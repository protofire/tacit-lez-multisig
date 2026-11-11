//! Private-posture e2e against a standalone LEZ v0.4 sequencer through the (patched) LEZ wallet:
//! deploy at `P`, GMS group, shared private PDA `V` bound to `P`, `Create`, fund `V`, M-of-N
//! `Execute` (native transfer to a public account), sub-quorum `Execute` rejected, replay
//! rejected, then a `Config` removing member 2: an approval signed before it (by the removed
//! member) and the removed member's fresh approval are both rejected, the remaining members
//! execute. Image v2: a spending limit is set and a `Spend` lands within it (an over-limit `Spend`
//! is rejected), then a time lock is set and a payout goes `Queue` -> wait for eta ->
//! `ExecuteQueued`.
//!
//! `msig-e2e payer-pubkey` prints the dev payer's public key (for the genesis config);
//! `msig-e2e run` runs the flow. Wallet home: `LEE_WALLET_HOME_DIR`. Dev keys only.

use anyhow::{Context, Result, anyhow, bail, ensure};
use common::HashType;
use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use lee_core::native_token::{NATIVE_TOKEN_PROGRAM_ID, decode_balance};
use lee_core::program::{PROGRAM_LOADER_ACCOUNT_ID, PdaSeed, ProgramHeader};
use common::transaction::LeeTransaction;
use msig_core::{Action, ConfigOp, Error, Instruction, Limit, Multisig, seed};
use msig_host::*;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use sha2::{Digest, Sha256};
use wallet::program_facades::CreditDelivery;
use wallet::program_facades::native_token_transfer::NativeTokenTransfer;
use wallet::program_facades::program_loader::ProgramLoader;
use wallet::{AccountIdentity, AccountMention, AsyncDelivery, WalletCore};

type Wallet = WalletCore<sequencer_service_rpc::SequencerClient>;

const M: u8 = 2;
const N: u8 = 3;
const FUND: u128 = 1_000;
const PAY: u128 = 300;
const LIMIT: u128 = 150;
const SPEND: u128 = 100;
const DAY_MS: u64 = 86_400_000;
const TIME_LOCK_MS: u64 = 5_000;

/// Deterministic local dev key from a label (never a real key).
fn dev_secret(label: &str) -> [u8; 32] {
    Sha256::digest(label.as_bytes()).into()
}

fn ef(e: impl std::fmt::Debug) -> anyhow::Error {
    anyhow!("{e:?}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn open() -> Result<Wallet> {
    let home = std::path::PathBuf::from(
        std::env::var("LEE_WALLET_HOME_DIR").context("LEE_WALLET_HOME_DIR")?,
    );
    let (cfg, st) = (home.join("wallet_config.json"), home.join("storage.json"));
    if st.exists() {
        return WalletCore::new_update_chain(cfg, st, None, "").map_err(ef);
    }
    let (w, _) = WalletCore::new_init_storage(cfg, st, None, "").map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    Ok(w)
}

/// Waits for inclusion; `Err` if the tx was reverted.
async fn included(w: &Wallet, hash: HashType, what: &str) -> Result<u64> {
    let (tx, block_id) = w.poll_transaction(hash).await.map_err(ef)?;
    let block = w
        .get_block(block_id)
        .await
        .map_err(ef)?
        .ok_or_else(|| anyhow!("block {block_id} missing"))?;
    let pos = block
        .body
        .transactions
        .iter()
        .position(|t| *t == tx)
        .ok_or_else(|| anyhow!("tx not in block"))?;
    ensure!(
        !block.body.is_reverted(pos),
        "{what}: tx {hash} REVERTED in block {block_id}"
    );
    let notes = match &tx {
        LeeTransaction::PrivacyPreserving(pp) => {
            let refund = pp.message.execution.fee.as_ref().map(|f| f.refund_to);
            ensure!(
                matches!(refund, None | Some(None)),
                "{what}: refund_to set: {refund:?}"
            );
            let lens: Vec<usize> = pp
                .message
                .execution
                .private_actions
                .iter()
                .map(|a| borsh::to_vec(&a.encrypted_post_state.ciphertext).unwrap().len() - 4)
                .collect();
            ensure!(
                !lens.is_empty() && lens.iter().all(|&l| l == msig_core::NOTE_PAD as usize),
                "{what}: private note lengths {lens:?} != NOTE_PAD {}",
                msig_core::NOTE_PAD
            );
            // the multisig program is claimed Undisclosed; neither P nor the image id is in the tx
            ensure!(
                pp.message.execution.program_image_claims.iter().all(|c| {
                    matches!(c, lee_core::ProgramImageClaim::Undisclosed { .. })
                }),
                "{what}: a Disclosed program image claim"
            );
            let raw = borsh::to_vec(&tx)?;
            let image: Vec<u8> = MSIG_ID.iter().flat_map(|x| x.to_le_bytes()).collect();
            for (n, needle) in [("P", program_account().value().to_vec()), ("image id", image)] {
                ensure!(
                    !raw.windows(32).any(|w| w == needle.as_slice()),
                    "{what}: {n} in the submitted tx"
                );
            }
            format!(", private notes {lens:?} B (== NOTE_PAD), refund_to None, program claim Undisclosed, no P/image id")
        }
        LeeTransaction::Public(_) => String::new(),
    };
    println!(
        "  {what}: tx {hash} included in block {block_id}, {} bytes (borsh){notes}",
        borsh::to_vec(&tx)?.len()
    );
    Ok(block_id)
}

async fn private(
    w: &mut Wallet,
    accounts: Vec<AccountMention>,
    ix: &Instruction,
    what: &str,
) -> Result<u64> {
    w.sync_to_latest_block().await.map_err(ef)?;
    // P's program as an Undisclosed claim: the header is immutable, its mirror commitment is in the
    // commitment set (the proof may sit under another root than the vault's: any known root passes)
    let header = ProgramHeader::from_bytes(
        w.get_account_view(Actor::new(program_account(), PROGRAM_LOADER_ACCOUNT_ID))
            .await
            .map_err(ef)?
            .ok_or_else(|| anyhow!("no header account at P"))?
            .data
            .actor_state(PROGRAM_LOADER_ACCOUNT_ID),
    )
    .ok_or_else(|| anyhow!("P holds no program header"))?;
    ensure!(header.immutable, "header at P is mutable: no Undisclosed claim");
    let (proofs, _, _) = w
        .get_proofs_and_root(&[mirror_commitment(&header)], vec![])
        .await
        .map_err(ef)?;
    let proof = proofs
        .into_iter()
        .next()
        .flatten()
        .ok_or_else(|| anyhow!("mirror commitment of P not in the commitment set"))?;
    let t = std::time::Instant::now();
    let (hash, _) = w
        .send_tx(
            accounts,
            0,
            borsh::to_vec(ix)?,
            &catalog_undisclosed(header, proof),
            None,
            AsyncDelivery::default(),
        )
        .await
        .map_err(ef)?;
    println!(
        "  {what}: simulated + proven + submitted in {:.1} s",
        t.elapsed().as_secs_f64()
    );
    let b = included(w, hash, what).await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    Ok(b)
}

/// `V`'s state as this wallet decrypted it: `(multisig, native balance)`.
fn vault(w: &Wallet, v: AccountId) -> Result<(Option<Multisig>, u128)> {
    let acct = &w
        .storage()
        .key_chain()
        .shared_private_account(v)
        .ok_or_else(|| anyhow!("V not tracked by the wallet"))?
        .account;
    let ms = acct.data.actor_state(program_account());
    let ms = (!ms.is_empty())
        .then(|| borsh::from_slice(ms))
        .transpose()?;
    Ok((
        ms,
        decode_balance(acct.data.actor_state(NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)?,
    ))
}

async fn public_balance(w: &Wallet, id: AccountId) -> Result<u128> {
    let a = w
        .get_account_view(Actor::native_balance(id))
        .await
        .map_err(ef)?
        .unwrap_or_default();
    decode_balance(a.data.actor_state(NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)
}

async fn run() -> Result<()> {
    let p = program_account();
    let payer_key = lee::PrivateKey::try_new(dev_secret("rfp005/dev-payer")).unwrap();
    let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&payer_key));
    let recipient = key_account(dev_secret("rfp005/dev-recipient"));
    let mut w = open()?;
    w.storage_mut()
        .key_chain_mut()
        .add_imported_public_account(payer_key);
    w.storage_mut()
        .key_chain_mut()
        .add_imported_public_account(lee::PrivateKey::try_new(deploy_key(MSIG_ID)).unwrap());
    w.store_persistent_data().map_err(ef)?;
    println!(
        "[0] sequencer head {}; payer {payer} balance {}",
        w.get_last_block_id().await.map_err(ef)?,
        public_balance(&w, payer).await?
    );

    // 1. deploy the guest at P (header account of the image-derived deploy key), immutable
    println!(
        "[1] deploy multisig image {} at P = {p}",
        hex::encode(risc0_zkvm::sha::Digest::from(MSIG_ID))
    );
    println!(
        "    (builtin address would be {}; not used)",
        AccountId::from_builtin_program(MSIG_ID)
    );
    if ProgramLoader(&mut w)
        .program(p)
        .await
        .map_err(ef)?
        .is_none()
    {
        let user_elf = risc0_binfmt::ProgramBinary::decode(MSIG_ELF)
            .map_err(ef)?
            .user_elf;
        let segs: Vec<AccountId> = std::iter::repeat_with(|| w.create_new_account_public(None).0)
            .take(user_elf.len().div_ceil(MAX_SEGMENT_DATA_LEN))
            .collect();
        w.store_persistent_data().map_err(ef)?;
        ProgramLoader(&mut w)
            .deploy(p, &segs, MSIG_ELF.to_vec(), true, Some(payer))
            .await
            .map_err(ef)?;
    }
    ensure!(
        ProgramLoader(&mut w)
            .program(p)
            .await
            .map_err(ef)?
            .is_some(),
        "program not deployed at P"
    );
    println!(
        "    deployed: {} segment(s)",
        risc0_binfmt::ProgramBinary::decode(MSIG_ELF)
            .map_err(ef)?
            .user_elf
            .len()
            .div_ceil(MAX_SEGMENT_DATA_LEN)
    );

    // fee payer of the private txs: the submitter's own private account (refund_to stays None)
    let (submitter, _) = w.create_new_account_private(None);
    w.store_persistent_data().map_err(ef)?;
    w.sync_to_latest_block().await.map_err(ef)?;
    let (hash, _) = NativeTokenTransfer(&w)
        .transfer(
            AccountIdentity::Public(payer),
            AccountIdentity::PrivateOwned(submitter),
            100_000_000_000,
            CreditDelivery::Automatic,
        )
        .await
        .map_err(ef)?;
    included(
        &w,
        hash,
        "    shield fee funds to the submitter's private account",
    )
    .await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.set_private_fee_payer(Some(submitter));
    // the public recipient must exist before a private tx can name it (else UnadmittedPublicActor)
    let (hash, _) = NativeTokenTransfer(&w)
        .transfer(
            AccountIdentity::Public(payer),
            AccountIdentity::PublicForeign(lee::PublicKey::new_from_private_key(
                &lee::PrivateKey::try_new(dev_secret("rfp005/dev-recipient")).unwrap(),
            )),
            1,
            CreditDelivery::Automatic,
        )
        .await
        .map_err(ef)?;
    included(&w, hash, "    open recipient public account (1 unit)").await?;
    w.store_persistent_data().map_err(ef)?;

    // 2. GMS group and the shared private PDA V bound to P (patched wallet helper)
    let cfg = dev_cfg(N, M);
    let salt: [u8; 32] = dev_secret(&format!("rfp005/e2e-salt/{}", now_ms()));
    let s = seed(&cfg, &salt);
    let group = wallet::account::Label::new("msig-e2e");
    w.insert_group_key_holder(group.clone(), GroupKeyHolder::new(), true);
    let info = w
        .create_shared_pda_account_at(group, PdaSeed::new(s), MSIG_ID, p)
        .await
        .map_err(ef)?;
    let v = info.account_id;
    ensure!(
        v == AccountId::for_private_pda(&p, &PdaSeed::new(s), &info.npk, &info.vpk),
        "V not bound to P"
    );
    println!(
        "[2] {N} dev members, threshold M={M}; private PDA V = {v} (seed commits to cfg, chain_tag, salt)"
    );

    // 3. Create
    let me = AccountIdentity::PrivateOwned(v);
    let create = Instruction::Create {
        cfg: cfg.clone(),
        salt,
        keys: Some((info.npk, info.vpk)),
    };
    private(
        &mut w,
        vec![me.clone().select_program_actor_state(p)],
        &create,
        "[3] Create",
    )
    .await?;
    let (ms, bal) = vault(&w, v)?;
    let ms = ms.ok_or_else(|| anyhow!("V has no multisig state after Create"))?;
    println!(
        "    V state: threshold {} of {} members, epoch {}, nonce {}; balance {bal}",
        ms.threshold,
        ms.members.len(),
        ms.config_epoch,
        ms.nonce
    );

    // 4. fund V from the public payer (credit applied to V in the same tx)
    w.sync_to_latest_block().await.map_err(ef)?;
    let (hash, _) = NativeTokenTransfer(&w)
        .transfer(
            AccountIdentity::Public(payer),
            me.clone(),
            FUND,
            CreditDelivery::Automatic,
        )
        .await
        .map_err(ef)?;
    included(&w, hash, "[4] fund V").await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    let (_, bal) = vault(&w, v)?;
    ensure!(bal == FUND, "V balance {bal} after funding");
    println!("    V balance (decrypted by a GMS holder): {bal}");

    // 5. sub-quorum Execute (M-1 = 1 approval): the program rejects it while proving, nothing lands
    let to = AccountIdentity::PublicNoSign(recipient)
        .balance()
        .receiving();
    let expiry = now_ms() + 3_600_000;
    let sub = signed_execute(v, &ms, native_transfer(v, recipient, PAY), expiry, &[1]);
    match private(
        &mut w,
        vec![me.clone().select_program_actor_state(p), to.clone()],
        &sub,
        "[5] Execute with 1 approval",
    )
    .await
    {
        Ok(_) => bail!("sub-quorum Execute was accepted"),
        Err(e) => {
            let s = format!("{e:#}");
            ensure!(
                s.contains("ProgramExitedWithCode")
                    && s.contains(&format!("code: {}", msig_core::Error::BelowThreshold as u8)),
                "unexpected failure: {s}"
            );
            println!(
                "[5] Execute with 1 of M={M} approvals REJECTED: guest exit code {} (BelowThreshold); no tx submitted",
                msig_core::Error::BelowThreshold as u8
            );
            println!("    error: {}", s.chars().take(300).collect::<String>());
        }
    }
    let (_, bal) = vault(&w, v)?;
    ensure!(
        bal == FUND && public_balance(&w, recipient).await? == 1,
        "funds moved by the sub-quorum attempt"
    );
    println!("    after rejection: V balance {bal}, recipient {recipient} balance 1 (unchanged)");

    // 6. M-of-N Execute: native transfer V -> recipient under the vault seed grant
    let ix = signed_execute(v, &ms, native_transfer(v, recipient, PAY), expiry, &[1, 2]);
    private(
        &mut w,
        vec![me.select_program_actor_state(p), to],
        &ix,
        "[6] Execute with 2 of 3 approvals",
    )
    .await?;
    let (ms2, bal) = vault(&w, v)?;
    let got = public_balance(&w, recipient).await?;
    println!(
        "    V balance {bal}, nonce {}; recipient balance {got}",
        ms2.as_ref().map_or(0, |m| m.nonce)
    );
    ensure!(
        bal == FUND - PAY && got == 1 + PAY,
        "unexpected balances after Execute"
    );

    // 7. replay of the executed approvals at the new nonce
    match private(
        &mut w,
        vec![
            AccountIdentity::PrivateOwned(v).select_program_actor_state(p),
            AccountIdentity::PublicNoSign(recipient)
                .balance()
                .receiving(),
        ],
        &ix,
        "[7] replay",
    )
    .await
    {
        Ok(_) => bail!("replayed Execute was accepted"),
        Err(e) => {
            let s = format!("{e:#}");
            ensure!(
                s.contains("ProgramExitedWithCode") && s.contains(&format!("code: {}", Error::BadSignature as u8)),
                "[7] unexpected failure: {s}"
            );
            println!("[7] replay of the same approvals at nonce 1 REJECTED, guest exit code {} (BadSignature)", Error::BadSignature as u8);
        }
    }

    // 8. Config: remove member 2 (quorum 1+2). Before it lands, member 2 also approves a payout
    //    for the next nonce at the current epoch: a stale approval once the epoch moves.
    let ms1 = ms2.ok_or_else(|| anyhow!("V has no multisig state"))?;
    let removed = ms1.members[2].key;
    let stale = signed_execute(
        v,
        &Multisig {
            nonce: ms1.nonce + 1,
            ..ms1.clone()
        },
        native_transfer(v, recipient, PAY),
        expiry,
        &[1, 2],
    );
    let remove = Action::Config(ConfigOp::RemoveMember { key: removed });
    let vault_accts = || {
        vec![
            AccountIdentity::PrivateOwned(v).select_program_actor_state(p),
            AccountIdentity::PublicNoSign(recipient)
                .balance()
                .receiving(),
        ]
    };
    private(
        &mut w,
        vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p)],
        &signed_execute(v, &ms1, remove, expiry, &[1, 2]),
        "[8] Config RemoveMember(member 2) with 2 of 3 approvals",
    )
    .await?;
    let (ms3, bal) = vault(&w, v)?;
    let ms3 = ms3.ok_or_else(|| anyhow!("V has no multisig state"))?;
    ensure!(
        ms3.members.len() == 2 && ms3.config_epoch == 1 && ms3.nonce == ms1.nonce + 1,
        "unexpected state after Config"
    );
    println!(
        "    V state: threshold {} of {} members, epoch {}, nonce {}; balance {bal}",
        ms3.threshold,
        ms3.members.len(),
        ms3.config_epoch,
        ms3.nonce
    );

    // 9 and 10: both must fail in the guest with the given code; nothing is submitted
    for (step, ix, code, what) in [
        (
            "[9] stale approvals (epoch 0, incl. removed member 2) for nonce 2",
            stale,
            Error::BadSignature,
            "BadSignature",
        ),
        (
            "[10] removed member 2 approves afresh at epoch 1",
            signed_execute(v, &ms3, native_transfer(v, recipient, PAY), expiry, &[1, 2]),
            Error::NotMember,
            "NotMember",
        ),
    ] {
        match private(&mut w, vault_accts(), &ix, step).await {
            Ok(_) => bail!("{step}: accepted"),
            Err(e) => {
                let s = format!("{e:#}");
                ensure!(
                    s.contains("ProgramExitedWithCode") && s.contains(&format!("code: {}", code as u8)),
                    "{step}: unexpected failure: {s}"
                );
                println!("{step}: REJECTED, guest exit code {} ({what}); no tx submitted", code as u8);
            }
        }
    }

    // 11. the remaining members execute at epoch 1
    private(
        &mut w,
        vault_accts(),
        &signed_execute(v, &ms3, native_transfer(v, recipient, PAY), expiry, &[0, 1]),
        "[11] Execute with remaining members 0 and 1 at epoch 1",
    )
    .await?;
    let (_, bal) = vault(&w, v)?;
    let got = public_balance(&w, recipient).await?;
    println!("    V balance {bal}; recipient balance {got}");
    ensure!(
        bal == FUND - 2 * PAY && got == 1 + 2 * PAY,
        "unexpected balances after the epoch-1 Execute"
    );

    // 12. Config SetLimit: LIMIT per day, 1 SPEND signature (immediate: no time lock yet)
    let (ms4, _) = vault(&w, v)?;
    let ms4 = ms4.ok_or_else(|| anyhow!("V has no multisig state"))?;
    let set_limit = Action::Config(ConfigOp::SetLimit(Some(Limit {
        amount: LIMIT,
        period: DAY_MS,
        threshold: 1,
    })));
    private(
        &mut w,
        vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p)],
        &signed_execute(v, &ms4, set_limit, expiry, &[0, 1]),
        "[12] Config SetLimit(150 per day, 1 SPEND signature)",
    )
    .await?;
    let (ms5, _) = vault(&w, v)?;
    let ms5 = ms5.ok_or_else(|| anyhow!("V has no multisig state"))?;
    ensure!(ms5.limit.is_some() && ms5.config_epoch == 2, "limit not set");

    // 13. Spend SPEND by member 1 alone in today's period; 14. a second one exceeds the limit
    let day = now_ms() / DAY_MS;
    private(
        &mut w,
        vault_accts(),
        &signed_spend(v, &ms5, recipient, SPEND, day, &[1]),
        "[13] Spend 100 (member 1 alone, limit 150/day)",
    )
    .await?;
    let (ms6, bal) = vault(&w, v)?;
    let ms6 = ms6.ok_or_else(|| anyhow!("V has no multisig state"))?;
    let got = public_balance(&w, recipient).await?;
    println!(
        "    V balance {bal}; recipient balance {got}; spent {} in period {}, action nonce {} (unchanged)",
        ms6.spend.spent, ms6.spend.period_index, ms6.nonce
    );
    ensure!(
        bal == FUND - 2 * PAY - SPEND && got == 1 + 2 * PAY + SPEND && ms6.nonce == ms5.nonce,
        "unexpected state after Spend"
    );
    let step = "[14] second Spend 100 in the same period (200 > 150)";
    match private(&mut w, vault_accts(), &signed_spend(v, &ms6, recipient, SPEND, day, &[1]), step).await {
        Ok(_) => bail!("{step}: accepted"),
        Err(e) => {
            let s = format!("{e:#}");
            ensure!(
                s.contains("ProgramExitedWithCode") && s.contains(&format!("code: {}", Error::OverLimit as u8)),
                "{step}: unexpected failure: {s}"
            );
            println!("{step}: REJECTED, guest exit code {} (OverLimit); no tx submitted", Error::OverLimit as u8);
        }
    }

    // 15. Config SetTimeLock (immediate), 16. Queue a payout at t, 17. ExecuteQueued after eta
    private(
        &mut w,
        vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p)],
        &signed_execute(v, &ms6, Action::Config(ConfigOp::SetTimeLock(TIME_LOCK_MS)), expiry, &[0, 1]),
        "[15] Config SetTimeLock(5 s)",
    )
    .await?;
    let (ms7, _) = vault(&w, v)?;
    let ms7 = ms7.ok_or_else(|| anyhow!("V has no multisig state"))?;
    ensure!(ms7.time_lock == TIME_LOCK_MS, "time lock not set");
    let t = now_ms() + 30_000; // the Queue tx must land before t
    let pay = native_transfer(v, recipient, SPEND);
    private(
        &mut w,
        vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p)],
        &signed_queue(v, &ms7, pay.clone(), expiry, t, &[0, 1]),
        "[16] Queue payout 100 (t = now + 30 s, eta = t + 5 s)",
    )
    .await?;
    let (ms8, _) = vault(&w, v)?;
    let ms8 = ms8.ok_or_else(|| anyhow!("V has no multisig state"))?;
    ensure!(ms8.queued.as_ref().is_some_and(|q| q.t == t), "nothing queued");
    let eta = t + TIME_LOCK_MS;
    let wait = eta.saturating_sub(now_ms()) + 1_000;
    println!("    queued; waiting {:.1} s for eta", wait as f64 / 1e3);
    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    private(
        &mut w,
        vault_accts(),
        &signed_execute_queued(v, &ms8, pay),
        "[17] ExecuteQueued after eta",
    )
    .await?;
    let (ms9, bal) = vault(&w, v)?;
    let ms9 = ms9.ok_or_else(|| anyhow!("V has no multisig state"))?;
    let got = public_balance(&w, recipient).await?;
    println!(
        "    V balance {bal}; recipient balance {got}; nonce {}, queue slot {}",
        ms9.nonce,
        if ms9.queued.is_none() { "empty" } else { "FULL" }
    );
    ensure!(
        bal == FUND - 2 * PAY - 2 * SPEND && got == 1 + 2 * PAY + 2 * SPEND && ms9.queued.is_none(),
        "unexpected state after ExecuteQueued"
    );
    println!("PASS");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("payer-pubkey") => {
            let k = lee::PrivateKey::try_new(dev_secret("rfp005/dev-payer")).unwrap();
            println!(
                "{}",
                hex::encode(lee::PublicKey::new_from_private_key(&k).value())
            );
            Ok(())
        }
        Some("run") => run().await,
        _ => bail!("usage: msig-e2e payer-pubkey | run"),
    }
}
