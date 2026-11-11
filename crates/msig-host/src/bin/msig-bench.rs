//! Benchmark client against a standalone LEZ v0.4 sequencer (scripts/bench-gpu.sh), real fee path
//! (private fee payer, fee inbox) and the Undisclosed program claim, as msig-e2e. One subcommand
//! per process so /usr/bin/time and the GPU sampler measure exactly one operation:
//!   payer-pubkey | init | create M N LABEL | fund LABEL AMOUNT | exec LABEL
//! Env: LEE_WALLET_HOME_DIR (wallet + bench state), BENCH_MODE=real|dev (must match RISC0_DEV_MODE).
//! Each op prints one `RESULT` line. Timings: sync, dry-run (pre-prove part), send_tx (simulate +
//! price + PROVE + sign + submit), inclusion poll. Dev keys only.

use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};
use common::HashType;
use common::transaction::LeeTransaction;
use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use lee_core::native_token::decode_balance;
use lee_core::program::{PROGRAM_LOADER_ACCOUNT_ID, PdaSeed, ProgramHeader};
use msig_core::{Instruction, Multisig, seed};
use msig_host::*;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use sha2::{Digest, Sha256};
use wallet::program_facades::CreditDelivery;
use wallet::program_facades::native_token_transfer::NativeTokenTransfer;
use wallet::program_facades::program_loader::ProgramLoader;
use wallet::{AccountIdentity, AsyncDelivery, WalletCore};

type Wallet = WalletCore<sequencer_service_rpc::SequencerClient>;

const PAY: u128 = 1;

fn dev_secret(label: &str) -> [u8; 32] {
    Sha256::digest(label.as_bytes()).into()
}
fn ef(e: impl std::fmt::Debug) -> anyhow::Error {
    anyhow!("{e:?}")
}
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}
fn home() -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(std::env::var("LEE_WALLET_HOME_DIR").context("LEE_WALLET_HOME_DIR")?))
}
fn dev() -> bool {
    std::env::var("RISC0_DEV_MODE").is_ok()
}
fn id_hex(a: AccountId) -> String {
    hex::encode(a.value())
}
fn id_from(s: &str) -> Result<AccountId> {
    Ok(AccountId::new(hex::decode(s.trim())?.try_into().map_err(|_| anyhow!("bad id"))?))
}

fn open() -> Result<Wallet> {
    let h = home()?;
    let (cfg, st) = (h.join("wallet_config.json"), h.join("storage.json"));
    if st.exists() {
        let mut w = WalletCore::new_update_chain(cfg, st, None, "").map_err(ef)?;
        if let Ok(s) = std::fs::read_to_string(h.join("submitter")) {
            w.set_private_fee_payer(Some(id_from(&s)?));
        }
        return Ok(w);
    }
    let (w, _) = WalletCore::new_init_storage(cfg, st, None, "").map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    Ok(w)
}

struct Incl {
    block: u64,
    tx_bytes: usize,
    proof_bytes: usize,
    secs: f64,
}

/// Waits for inclusion (not reverted) and re-asserts the e2e's tx-shape checks.
async fn included(w: &Wallet, hash: HashType, what: &str, check_shape: bool) -> Result<Incl> {
    let t = Instant::now();
    let (tx, block_id) = w.poll_transaction(hash).await.map_err(ef)?;
    let secs = t.elapsed().as_secs_f64();
    let block = w.get_block(block_id).await.map_err(ef)?.ok_or_else(|| anyhow!("block {block_id} missing"))?;
    let pos = block.body.transactions.iter().position(|t| *t == tx).ok_or_else(|| anyhow!("tx not in block"))?;
    ensure!(!block.body.is_reverted(pos), "{what}: tx {hash} REVERTED in block {block_id}");
    let raw = borsh::to_vec(&tx)?;
    let mut proof_bytes = 0;
    if let LeeTransaction::PrivacyPreserving(pp) = &tx {
        proof_bytes = borsh::to_vec(pp.witness_set.proof())?.len() - 4;
        ensure!(matches!(pp.message.execution.fee.as_ref().map(|f| f.refund_to), None | Some(None)), "{what}: refund_to set");
        if check_shape {
            let lens: Vec<usize> = pp.message.execution.private_actions.iter()
                .map(|a| borsh::to_vec(&a.encrypted_post_state.ciphertext).unwrap().len() - 4).collect();
            ensure!(!lens.is_empty() && lens.iter().all(|&l| l == msig_core::NOTE_PAD as usize), "{what}: note lengths {lens:?}");
            ensure!(pp.message.execution.program_image_claims.iter()
                .all(|c| matches!(c, lee_core::ProgramImageClaim::Undisclosed { .. })), "{what}: Disclosed program claim");
            let image: Vec<u8> = MSIG_ID.iter().flat_map(|x| x.to_le_bytes()).collect();
            for needle in [program_account().value().to_vec(), image] {
                ensure!(!raw.windows(32).any(|w| w == needle.as_slice()), "{what}: P or image id in the tx");
            }
        }
    }
    Ok(Incl { block: block_id, tx_bytes: raw.len(), proof_bytes, secs })
}

/// sync, then the same call as a dry run (everything before proving), then for real, then wait for
/// inclusion. `$call` is evaluated twice and must end in `.await`.
macro_rules! timed {
    ($w:ident, $op:expr, $extra:expr, $shape:expr, $call:expr) => {{
        let t = Instant::now();
        $w.sync_to_latest_block().await.map_err(ef)?;
        let sync_s = t.elapsed().as_secs_f64();
        $w.set_dry_run(true);
        let t = Instant::now();
        let dry = $call;
        let dry_s = t.elapsed().as_secs_f64();
        $w.set_dry_run(false);
        ensure!(matches!(&dry, Err(e) if format!("{e:?}").contains("DryRun")), "dry run did not end in DryRun");
        let t = Instant::now();
        let (hash, _) = $call.map_err(ef)?;
        let send_s = t.elapsed().as_secs_f64();
        let i = included(&$w, hash, $op, $shape).await?;
        $w.sync_to_latest_block().await.map_err(ef)?;
        $w.store_persistent_data().map_err(ef)?;
        println!(
            "RESULT op={} {} sync_s={sync_s:.2} dry_run_s={dry_s:.2} send_tx_s={send_s:.2} prove_est_s={:.2} incl_s={:.2} e2e_client_s={:.2} tx_bytes={} proof_bytes={} block={} tx={hash} image_id={} dev_mode={}",
            $op, $extra, send_s - dry_s, i.secs, sync_s + send_s + i.secs, i.tx_bytes, i.proof_bytes, i.block,
            msig_image_hex(), dev()
        );
    }};
}

async fn programs(w: &Wallet) -> Result<lee::privacy_preserving_transaction::circuit::ProgramCatalog> {
    let p = program_account();
    let header = ProgramHeader::from_bytes(
        w.get_account_view(Actor::new(p, PROGRAM_LOADER_ACCOUNT_ID)).await.map_err(ef)?
            .ok_or_else(|| anyhow!("no header at P"))?.data.actor_state(PROGRAM_LOADER_ACCOUNT_ID),
    ).ok_or_else(|| anyhow!("P holds no program header"))?;
    ensure!(header.immutable, "header at P is mutable");
    let (proofs, _, _) = w.get_proofs_and_root(&[mirror_commitment(&header)], vec![]).await.map_err(ef)?;
    let proof = proofs.into_iter().next().flatten().ok_or_else(|| anyhow!("mirror commitment not in set"))?;
    Ok(catalog_undisclosed(header, proof))
}

fn vault(w: &Wallet, v: AccountId) -> Result<(Option<Multisig>, u128)> {
    let acct = &w.storage().key_chain().shared_private_account(v).ok_or_else(|| anyhow!("V not tracked"))?.account;
    let ms = acct.data.actor_state(program_account());
    let ms = (!ms.is_empty()).then(|| borsh::from_slice(ms)).transpose()?;
    Ok((ms, decode_balance(acct.data.actor_state(lee_core::native_token::NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)?))
}

async fn public_balance(w: &Wallet, id: AccountId) -> Result<u128> {
    let a = w.get_account_view(Actor::native_balance(id)).await.map_err(ef)?.unwrap_or_default();
    decode_balance(a.data.actor_state(lee_core::native_token::NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)
}

fn recipient() -> AccountId {
    key_account(dev_secret("rfp005/dev-recipient"))
}

/// `(v, m, n, salt)` of a bench vault from `$HOME/vault-LABEL`.
fn load(label: &str) -> Result<(AccountId, u8, u8, [u8; 32])> {
    let s = std::fs::read_to_string(home()?.join(format!("vault-{label}"))).with_context(|| format!("vault {label}"))?;
    let f: Vec<&str> = s.split_whitespace().collect();
    Ok((id_from(f[0])?, f[1].parse()?, f[2].parse()?, hex::decode(f[3])?.try_into().map_err(|_| anyhow!("salt"))?))
}

async fn run(args: Vec<String>) -> Result<()> {
    ensure!(msig_image_hex() == IMAGE_ID_V2_1, "msig image {} is not frozen v2.1: build with RISC0_USE_DOCKER=1", msig_image_hex());
    let mode = std::env::var("BENCH_MODE").context("BENCH_MODE=real|dev")?;
    ensure!((mode == "dev") == dev(), "BENCH_MODE={mode} but RISC0_DEV_MODE set = {}", dev());
    let p = program_account();
    let payer_key = lee::PrivateKey::try_new(dev_secret("rfp005/dev-payer")).unwrap();
    let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&payer_key));
    match args.get(1).map(String::as_str) {
        Some("init") => {
            let mut w = open()?;
            w.storage_mut().key_chain_mut().add_imported_public_account(payer_key);
            w.storage_mut().key_chain_mut().add_imported_public_account(lee::PrivateKey::try_new(deploy_key(MSIG_ID)).unwrap());
            w.store_persistent_data().map_err(ef)?;
            let user_elf = risc0_binfmt::ProgramBinary::decode(MSIG_ELF).map_err(ef)?.user_elf;
            let segs: Vec<AccountId> = std::iter::repeat_with(|| w.create_new_account_public(None).0)
                .take(user_elf.len().div_ceil(MAX_SEGMENT_DATA_LEN)).collect();
            w.store_persistent_data().map_err(ef)?;
            ProgramLoader(&mut w).deploy(p, &segs, MSIG_ELF.to_vec(), true, Some(payer)).await.map_err(ef)?;
            ensure!(ProgramLoader(&mut w).program(p).await.map_err(ef)?.is_some(), "not deployed at P");
            let (submitter, _) = w.create_new_account_private(None);
            w.store_persistent_data().map_err(ef)?;
            std::fs::write(home()?.join("submitter"), id_hex(submitter))?;
            w.sync_to_latest_block().await.map_err(ef)?;
            let (hash, _) = NativeTokenTransfer(&w)
                .transfer(AccountIdentity::Public(payer), AccountIdentity::PrivateOwned(submitter), 100_000_000_000, CreditDelivery::Automatic)
                .await.map_err(ef)?;
            let i = included(&w, hash, "shield fee funds", false).await?;
            w.sync_to_latest_block().await.map_err(ef)?;
            w.set_private_fee_payer(Some(submitter));
            let (hash, _) = NativeTokenTransfer(&w)
                .transfer(AccountIdentity::Public(payer), AccountIdentity::PublicForeign(lee::PublicKey::new_from_private_key(
                    &lee::PrivateKey::try_new(dev_secret("rfp005/dev-recipient")).unwrap())), 1, CreditDelivery::Automatic)
                .await.map_err(ef)?;
            included(&w, hash, "open recipient", false).await?;
            w.store_persistent_data().map_err(ef)?;
            println!("RESULT op=init deployed_at={p} image_id={} fee_shield_block={} dev_mode={}", msig_image_hex(), i.block, dev());
        }
        Some("create") => {
            let (m, n, label): (u8, u8, &str) = (args[2].parse()?, args[3].parse()?, &args[4]);
            let mut w = open()?;
            let cfg = dev_cfg(n, m);
            let salt = dev_secret(&format!("rfp005/bench-salt/{label}/{}", now_ms()));
            let s = seed(&cfg, &salt);
            let group = wallet::account::Label::new(&format!("msig-bench-{label}"));
            w.insert_group_key_holder(group.clone(), GroupKeyHolder::new(), true);
            let info = w.create_shared_pda_account_at(group, PdaSeed::new(s), MSIG_ID, p).await.map_err(ef)?;
            let v = info.account_id;
            ensure!(v == AccountId::for_private_pda(&p, &PdaSeed::new(s), &info.npk, &info.vpk), "V not bound to P");
            w.store_persistent_data().map_err(ef)?;
            std::fs::write(home()?.join(format!("vault-{label}")), format!("{} {m} {n} {}", id_hex(v), hex::encode(salt)))?;
            let cat = programs(&w).await?;
            let accts = vec![AccountIdentity::PrivateOwned(v).select_program_actor_state(p)];
            let msg = borsh::to_vec(&Instruction::Create { cfg, salt, keys: Some((info.npk, info.vpk)) })?;
            timed!(w, "create", format!("label={label} m={m} n={n}"), true,
                w.send_tx(accts.clone(), 0, msg.clone(), &cat, None, AsyncDelivery::default()).await);
            ensure!(vault(&w, v)?.0.is_some(), "no multisig state after Create");
        }
        Some("fund") => {
            let (label, amount): (&str, u128) = (&args[2], args[3].parse()?);
            let (v, m, n, _) = load(label)?;
            let mut w = open()?;
            timed!(w, "fund", format!("label={label} m={m} n={n} amount={amount}"), false,
                NativeTokenTransfer(&w).transfer(AccountIdentity::Public(payer), AccountIdentity::PrivateOwned(v), amount, CreditDelivery::Automatic).await);
            ensure!(vault(&w, v)?.1 >= amount, "V not funded");
        }
        Some("exec") => {
            let (v, m, n, _) = load(&args[2])?;
            let mut w = open()?;
            w.sync_to_latest_block().await.map_err(ef)?;
            let (ms, bal0) = vault(&w, v)?;
            let ms = ms.ok_or_else(|| anyhow!("V has no state"))?;
            let to = recipient();
            let got0 = public_balance(&w, to).await?;
            let approvers: Vec<u8> = (0..m).collect();
            let ix = signed_execute(v, &ms, native_transfer(v, to, PAY), now_ms() + 3_600_000, &approvers);
            let cat = programs(&w).await?;
            let accts = vec![
                AccountIdentity::PrivateOwned(v).select_program_actor_state(p),
                AccountIdentity::PublicNoSign(to).balance().receiving(),
            ];
            let msg = borsh::to_vec(&ix)?;
            timed!(w, "exec", format!("label={} m={m} n={n} sigs={}", args[2], m + 2), true,
                w.send_tx(accts.clone(), 0, msg.clone(), &cat, None, AsyncDelivery::default()).await);
            let (ms2, bal1) = vault(&w, v)?;
            let got1 = public_balance(&w, to).await?;
            ensure!(bal1 == bal0 - PAY && got1 == got0 + PAY && ms2.map(|x| x.nonce) == Some(ms.nonce + 1), "balances/nonce unchanged: V {bal0}->{bal1}, recipient {got0}->{got1}");
            println!("CHECK vault_balance {bal0}->{bal1} recipient_balance {got0}->{got1} nonce {}->{}", ms.nonce, ms.nonce + 1);
        }
        _ => bail!("usage: payer-pubkey | init | create M N LABEL | fund LABEL AMOUNT | exec LABEL"),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init(); // silent unless RUST_LOG is set
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("payer-pubkey") {
        let k = lee::PrivateKey::try_new(dev_secret("rfp005/dev-payer")).unwrap();
        println!("{}", hex::encode(lee::PublicKey::new_from_private_key(&k).value()));
        return Ok(());
    }
    run(args).await
}
