//! Plain-Rust `create` and `execute` of the private multisig over a LEZ v0.4 sequencer, host-testable
//! (no Logos types). Same flow as crates/msig-host/src/bin/msig-e2e.rs steps 1-4 and 6, split in two
//! calls that share a wallet home. Dev keys only: payer, members and recipient derive from public labels.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use common::HashType;
use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use lee_core::native_token::{NATIVE_TOKEN_PROGRAM_ID, decode_balance};
use lee_core::program::PdaSeed;
use msig_core::{Instruction, Multisig, seed};
use msig_host::*;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wallet::program_facades::CreditDelivery;
use wallet::program_facades::native_token_transfer::NativeTokenTransfer;
use wallet::program_facades::program_loader::ProgramLoader;
use wallet::{AccountIdentity, AccountMention, AsyncDelivery, WalletCore};

type Wallet = WalletCore<sequencer_service_rpc::SequencerClient>;

const STATE: &str = "msig.json";
const GROUP: &str = "msig-module";
/// Native units shielded to the submitter's private account to pay fees of the private txs.
const FEE_FUNDS: u128 = 100_000_000_000;

fn dev_secret(label: &str) -> [u8; 32] {
    Sha256::digest(label.as_bytes()).into()
}

fn ef(e: impl std::fmt::Debug) -> anyhow::Error {
    anyhow!("{e:?}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn payer_key() -> lee::PrivateKey {
    lee::PrivateKey::try_new(dev_secret("rfp005/dev-payer")).unwrap()
}

fn open(home: &Path) -> Result<Wallet> {
    let (cfg, st) = (home.join("wallet_config.json"), home.join("storage.json"));
    ensure!(cfg.exists(), "{}: missing (sequencer address config)", cfg.display());
    let mut w = if st.exists() {
        WalletCore::new_update_chain(cfg, st, None, "").map_err(ef)?
    } else {
        let (w, _) = WalletCore::new_init_storage(cfg, st, None, "").map_err(ef)?;
        w.store_persistent_data().map_err(ef)?;
        w
    };
    w.storage_mut().key_chain_mut().add_imported_public_account(payer_key());
    Ok(w)
}

/// Waits for inclusion in a block; `Err` if the tx was reverted. Returns the block id.
async fn included(w: &Wallet, hash: HashType, what: &str) -> Result<u64> {
    let (tx, block_id) = w.poll_transaction(hash).await.map_err(ef)?;
    let block = w.get_block(block_id).await.map_err(ef)?.ok_or_else(|| anyhow!("block {block_id} missing"))?;
    let pos = block.body.transactions.iter().position(|t| *t == tx).ok_or_else(|| anyhow!("tx not in block"))?;
    ensure!(!block.body.is_reverted(pos), "{what}: tx {hash} REVERTED in block {block_id}");
    Ok(block_id)
}

async fn private(w: &mut Wallet, accounts: Vec<AccountMention>, ix: &Instruction, what: &str) -> Result<u64> {
    w.sync_to_latest_block().await.map_err(ef)?;
    let (hash, _) = w
        .send_tx(accounts, 0, borsh::to_vec(ix)?, &catalog(), None, AsyncDelivery::default())
        .await
        .map_err(ef)?;
    let b = included(w, hash, what).await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    Ok(b)
}

async fn public_transfer(w: &Wallet, to: AccountIdentity, amount: u128, what: &str) -> Result<u64> {
    let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&payer_key()));
    let (hash, _) = NativeTokenTransfer(w)
        .transfer(AccountIdentity::Public(payer), to, amount, CreditDelivery::Automatic)
        .await
        .map_err(ef)?;
    included(w, hash, what).await
}

/// `V`'s state as this wallet decrypted it: `(multisig, native balance)`.
fn vault(w: &Wallet, v: AccountId) -> Result<(Multisig, u128)> {
    let acct = &w.storage().key_chain().shared_private_account(v).ok_or_else(|| anyhow!("V not tracked by the wallet"))?.account;
    let ms = borsh::from_slice(acct.data.actor_state(program_account())).context("V has no multisig state")?;
    Ok((ms, decode_balance(acct.data.actor_state(NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)?))
}

async fn public_balance(w: &Wallet, id: AccountId) -> Result<u128> {
    let a = w.get_account_view(Actor::native_balance(id)).await.map_err(ef)?.unwrap_or_default();
    decode_balance(a.data.actor_state(NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread().enable_all().build()?)
}

/// Writes the wallet-home state file atomically (a crash leaves the old or the new file, never half).
fn save(home: &Path, st: &Value) -> Result<()> {
    let tmp = home.join(format!("{STATE}.tmp"));
    std::fs::write(&tmp, st.to_string())?;
    Ok(std::fs::rename(tmp, home.join(STATE))?)
}

fn done(r: Result<Value>) -> Value {
    r.unwrap_or_else(|e| json!({ "ok": false, "error": format!("{e:#}") }))
}

pub fn create(home: &str, members: i64, threshold: i64, fund: &str) -> Value {
    done((|| {
        let home = PathBuf::from(home);
        let (n, m) = (u8::try_from(members).context("members: 1..=255")?, u8::try_from(threshold).context("threshold: 1..=255")?);
        ensure!(n >= 1 && m >= 1 && m <= n, "need 1 <= threshold <= members");
        let fund: u128 = fund.parse().context("fund: decimal u128")?;
        if let Ok(old) = std::fs::read_to_string(home.join(STATE)) {
            anyhow::bail!("{STATE} exists in the wallet home ({old}): one multisig per home");
        }
        runtime()?.block_on(async {
            let p = program_account();
            let mut w = open(&home)?;
            let group = wallet::account::Label::new(GROUP);
            // V's keys derive from this group's secret: replacing it strands a vault an earlier, interrupted create made
            ensure!(
                w.storage().key_chain().group_key_holder(&group).is_none(),
                "group '{GROUP}' already in this wallet home (an earlier create got that far, no {STATE}): refusing to replace it; its vault keeps its keys",
            );
            w.storage_mut().key_chain_mut().add_imported_public_account(lee::PrivateKey::try_new(deploy_key(MSIG_ID)).unwrap());
            w.store_persistent_data().map_err(ef)?;

            // 1. deploy the guest at P (immutable), unless already there
            if ProgramLoader(&mut w).program(p).await.map_err(ef)?.is_none() {
                let user_elf = risc0_binfmt::ProgramBinary::decode(MSIG_ELF).map_err(ef)?.user_elf;
                let segs: Vec<AccountId> = std::iter::repeat_with(|| w.create_new_account_public(None).0)
                    .take(user_elf.len().div_ceil(MAX_SEGMENT_DATA_LEN))
                    .collect();
                w.store_persistent_data().map_err(ef)?;
                let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&payer_key()));
                ProgramLoader(&mut w).deploy(p, &segs, MSIG_ELF.to_vec(), true, Some(payer)).await.map_err(ef)?;
            }
            ensure!(ProgramLoader(&mut w).program(p).await.map_err(ef)?.is_some(), "program not deployed at P");

            // fee payer of the private txs: the submitter's own private account (refund_to stays None)
            let (submitter, _) = w.create_new_account_private(None);
            w.store_persistent_data().map_err(ef)?;
            w.sync_to_latest_block().await.map_err(ef)?;
            public_transfer(&w, AccountIdentity::PrivateOwned(submitter), FEE_FUNDS, "shield fee funds").await?;
            w.sync_to_latest_block().await.map_err(ef)?;
            w.set_private_fee_payer(Some(submitter));

            // 2. GMS group and the shared private PDA V bound to P
            let cfg = dev_cfg(n, m);
            let salt: [u8; 32] = dev_secret(&format!("rfp005/module-salt/{}", now_ms()));
            w.insert_group_key_holder(group.clone(), GroupKeyHolder::new(), true);
            let info = w.create_shared_pda_account_at(group, PdaSeed::new(seed(&cfg, &salt)), MSIG_ID, p).await.map_err(ef)?;
            let v = info.account_id;
            // persist the group and the vault marker before any tx touches V; `funded` is set after funding
            w.store_persistent_data().map_err(ef)?;
            let mut st = json!({ "vault": v.to_string(), "submitter": submitter.to_string(), "funded": false });
            save(&home, &st)?;

            // 3. Create
            let me = AccountIdentity::PrivateOwned(v);
            let create = Instruction::Create { cfg, salt, keys: Some((info.npk, info.vpk)) };
            let block = private(&mut w, vec![me.clone().select_program_actor_state(p)], &create, "Create").await?;
            let (ms, _) = vault(&w, v)?;

            // 4. fund V from the public payer (credit applied to V in the same tx)
            let mut fund_block = Value::Null;
            if fund > 0 {
                w.sync_to_latest_block().await.map_err(ef)?;
                fund_block = public_transfer(&w, me, fund, "fund V").await?.into();
                w.sync_to_latest_block().await.map_err(ef)?;
            }
            w.store_persistent_data().map_err(ef)?;
            let (_, balance) = vault(&w, v)?;
            ensure!(balance == fund, "V balance {balance} after funding, expected {fund}");
            st["funded"] = true.into();
            save(&home, &st)?;
            Ok(json!({
                "ok": true, "program": p.to_string(), "vault": v.to_string(), "createBlock": block, "fundBlock": fund_block,
                "threshold": ms.threshold, "members": ms.members.len(), "epoch": ms.config_epoch, "nonce": ms.nonce, "balance": balance.to_string(),
            }))
        })
    })())
}

pub fn execute(home: &str, recipient_label: &str, amount: &str, signers: &str) -> Value {
    done((|| {
        let home = PathBuf::from(home);
        let amount: u128 = amount.parse().context("amount: decimal u128")?;
        let signers = signers.split(',').map(|s| s.trim().parse::<u8>()).collect::<Result<Vec<_>, _>>().context("signers: e.g. 1,2")?;
        let mut st: Value = serde_json::from_slice(&std::fs::read(home.join(STATE)).context("no multisig in this wallet home: run create first")?)?;
        let id = |k: &str| -> Result<AccountId> { st[k].as_str().context(k.to_string())?.parse().map_err(|e| anyhow!("{k}: {e:?}")) };
        let (v, submitter) = (id("vault")?, id("submitter")?);
        let recipient = key_account(dev_secret(&format!("rfp005/module-recipient/{recipient_label}")));
        runtime()?.block_on(async {
            let p = program_account();
            let mut w = open(&home)?;
            w.sync_to_latest_block().await.map_err(ef)?;
            w.set_private_fee_payer(Some(submitter));
            let (ms, vault_before) = vault(&w, v)?;
            // An earlier execute submitted but did not confirm: its signed nonce decides. Consumed -> it (or a same-nonce
            // retry of it) landed, never sign again. Not consumed -> any new signature reuses that nonce, so at most one lands.
            let pending = st["pending"].take();
            if !pending.is_null() && ms.nonce > pending["nonce"].as_u64().context("pending.nonce")? {
                save(&home, &st)?;
                let same = pending["recipient"] == recipient.to_string() && pending["amount"] == amount.to_string();
                ensure!(same, "an earlier execute landed (pending {pending}); nothing sent now: call again for this payment");
                return Ok(json!({
                    "ok": true, "reconciled": pending, "nonce": ms.nonce, "vaultAfter": vault_before.to_string(),
                    "recipient": recipient.to_string(), "recipientAfter": public_balance(&w, recipient).await?.to_string(),
                }));
            }
            // the public recipient must exist before a private tx can name it (else UnadmittedPublicActor)
            if public_balance(&w, recipient).await? == 0 {
                let pk = lee::PublicKey::new_from_private_key(&lee::PrivateKey::try_new(dev_secret(&format!("rfp005/module-recipient/{recipient_label}"))).unwrap());
                public_transfer(&w, AccountIdentity::PublicForeign(pk), 1, "open recipient").await?;
            }
            let before = public_balance(&w, recipient).await?;
            let ix = signed_execute(v, &ms, native_transfer(v, recipient, amount), now_ms() + 3_600_000, &signers);
            let to = AccountIdentity::PublicNoSign(recipient).balance().receiving();
            st["pending"] = json!({ "nonce": ms.nonce, "recipient": recipient.to_string(), "amount": amount.to_string() });
            save(&home, &st)?;
            w.sync_to_latest_block().await.map_err(ef)?;
            let (hash, _) = w
                .send_tx(vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p), to], 0, borsh::to_vec(&ix)?, &catalog(), None, AsyncDelivery::default())
                .await
                .map_err(ef)?;
            st["pending"]["hash"] = hash.to_string().into();
            save(&home, &st)?;
            let block = included(&w, hash, "Execute").await?;
            w.sync_to_latest_block().await.map_err(ef)?;
            w.store_persistent_data().map_err(ef)?;
            st["pending"].take();
            save(&home, &st)?;
            let (ms2, vault_after) = vault(&w, v)?;
            let after = public_balance(&w, recipient).await?;
            ensure!(vault_after == vault_before - amount && after == before + amount, "unexpected balances after Execute");
            Ok(json!({
                "ok": true, "block": block, "approvals": signers.len(), "threshold": ms.threshold, "nonce": ms2.nonce,
                "vaultBefore": vault_before.to_string(), "vaultAfter": vault_after.to_string(),
                "recipient": recipient.to_string(), "recipientBefore": before.to_string(), "recipientAfter": after.to_string(),
            }))
        })
    })())
}
