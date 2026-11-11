//! `msig`: operator CLI for the RFP-005 private M-of-N multisig (docs/CLI.md).
//! Wallet home: `LEE_WALLET_HOME_DIR` (one per operator). Proposals travel as envelope files.

mod envelope;

use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::{Args, Parser, Subcommand};
use common::HashType;
use common::transaction::LeeTransaction;
use envelope::{Envelope, Registry, Role, render};
use k256::schnorr::SigningKey;
use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use lee_core::native_token::{Message as Native, NATIVE_TOKEN_PROGRAM_ID, decode_balance};
use lee_core::program::{PROGRAM_LOADER_ACCOUNT_ID, PdaSeed, ProgramHeader};
use msig_core::{
    Action, ConfigOp, EXECUTE, INITIATE, InitConfig, Instruction, Limit, Member, Multisig, SPEND, Sig,
    VOTE, digests, seed,
};
use msig_host::{MSIG_ELF, MSIG_ID, catalog_undisclosed, deploy_key, mirror_commitment, program_account};
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wallet::program_facades::CreditDelivery;
use wallet::program_facades::native_token_transfer::NativeTokenTransfer;
use wallet::program_facades::program_loader::ProgramLoader;
use wallet::{AccountIdentity, AccountMention, AsyncDelivery, WalletCore};

type Wallet = WalletCore<sequencer_service_rpc::SequencerClient>;

const GROUP: &str = "msig";
const FEE_FUNDS: u128 = 100_000_000_000;
/// `queue` sets `t = now + this`: the Queue tx must be included before `t` (dev-mode proving and a
/// 3 s block fit; a real proof needs a larger lead), and `eta = t + time lock`.
const QUEUE_LEAD_MS: u64 = 30_000;

#[derive(Parser)]
#[command(name = "msig", about = "RFP-005 private multisig operator CLI")]
struct Cli {
    /// Vault file (default: $LEE_WALLET_HOME_DIR/msig-vault.json)
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    /// Program registry JSON {"<program account id>": "name"} for `show`
    #[arg(long, global = true)]
    registry: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Prop {
    /// Proposer's member key file (hex secret)
    #[arg(long)]
    key: PathBuf,
    /// Envelope file to write
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 3600)]
    expiry_secs: u64,
}

#[derive(Subcommand)]
enum ConfigCmd {
    AddMember {
        /// x-only public key, hex
        pubkey: String,
        /// permission bits: 1 initiate, 2 vote, 4 execute, 8 spend
        #[arg(long, default_value_t = 7)]
        perms: u8,
        #[command(flatten)]
        p: Prop,
    },
    RemoveMember {
        pubkey: String,
        #[command(flatten)]
        p: Prop,
    },
    SetThreshold {
        threshold: u8,
        #[command(flatten)]
        p: Prop,
    },
    /// Time lock in ms (0 = immediate execution); with one set, actions go `queue` -> `execute-queued`
    SetTimeLock {
        ms: u64,
        #[command(flatten)]
        p: Prop,
    },
    /// Spending limit: `--threshold` spend members may move up to AMOUNT per `--period-ms`
    SetLimit {
        amount: u128,
        #[arg(long)]
        period_ms: u64,
        #[arg(long)]
        threshold: u8,
        #[command(flatten)]
        p: Prop,
    },
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a fresh member key (hex secret, mode 600) and print its public key
    Keygen { out: PathBuf },
    /// Init the wallet, deploy the multisig program if absent, make + fund a private fee account
    Setup {
        /// Funded public account key (hex secret) that pays the deploy and the fee funds
        #[arg(long)]
        payer_key: PathBuf,
    },
    /// Create the private multisig; writes the vault file other members `join`
    Create {
        /// Member public keys (hex), comma separated; each gets initiate+vote+execute
        #[arg(long, value_delimiter = ',', required = true)]
        members: Vec<String>,
        #[arg(long)]
        threshold: u8,
        /// Permission bits every member gets (1 initiate, 2 vote, 4 execute, 8 spend)
        #[arg(long, default_value_t = 7)]
        perms: u8,
        /// Hashed into the vault seed
        #[arg(long, default_value = "rfp005/local-dev")]
        chain_tag: String,
    },
    /// Import a vault file made by `create` (group secret + seed) into this wallet
    Join { file: PathBuf },
    /// Dev: pay native tokens from a public account to the vault, or open a public account
    Fund {
        amount: u128,
        #[arg(long)]
        payer_key: PathBuf,
        /// Instead of the vault: create/credit the public account of this key file
        #[arg(long)]
        to_key: Option<PathBuf>,
    },
    /// Members, threshold, epoch, nonce, balance as this wallet decrypts them
    Status,
    /// Propose a native transfer out of the vault
    Propose {
        /// Recipient account id (base58); must exist publicly
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u128,
        #[command(flatten)]
        p: Prop,
    },
    /// Propose a config change (same approve/execute flow)
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Spend envelope: native transfer under the spending limit, signed by you as a spend member
    /// (other spend members `approve` it; `execute` submits it; no proposal, no time lock)
    Spend {
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u128,
        /// Your member key file (needs the spend permission)
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Decode and print an envelope from its signed bytes (nothing else is trusted)
    Show { envelope: PathBuf },
    /// Add your approval signature
    Approve {
        envelope: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
    /// Add an advisory rejection (the program ignores it)
    Reject {
        envelope: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
    /// Collect approvals, prove and submit
    Execute {
        envelope: PathBuf,
        /// Executor's member key file (needs the execute permission)
        #[arg(long)]
        key: PathBuf,
    },
    /// Under a time lock: collect approvals, prove and submit `Queue` (eta = now + 30 s + time lock)
    Queue {
        envelope: PathBuf,
        /// The proposer's member key file (only the proposer queues: approvers do not sign t)
        #[arg(long)]
        key: PathBuf,
    },
    /// After eta: prove and submit `ExecuteQueued` of the queued envelope
    ExecuteQueued {
        envelope: PathBuf,
        /// Executor's member key file (needs the execute permission)
        #[arg(long)]
        key: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Execute,
    Queue,
    ExecuteQueued,
}

#[derive(Serialize, Deserialize)]
struct VaultFile {
    /// Group master secret (hex). Whoever holds this file can read the vault; keep it private.
    gms: String,
    seed: String,
    v: String,
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

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var("LEE_WALLET_HOME_DIR").context("set LEE_WALLET_HOME_DIR to this operator's wallet home")?,
    ))
}

fn random32() -> Result<[u8; 32]> {
    use std::io::Read;
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b)
}

fn read_secret(p: &Path) -> Result<[u8; 32]> {
    hex::decode(std::fs::read_to_string(p).with_context(|| p.display().to_string())?.trim())?
        .try_into()
        .map_err(|_| anyhow!("{}: expected 32 bytes of hex", p.display()))
}

fn read_key(p: &Path) -> Result<SigningKey> {
    SigningKey::from_bytes(&read_secret(p)?).map_err(|e| anyhow!("{}: {e}", p.display()))
}

fn pubkey32(sk: &SigningKey) -> [u8; 32] {
    sk.verifying_key().to_bytes().into()
}

fn write_secret(p: &Path, content: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?
        .write_all(content.as_bytes())?;
    Ok(())
}

fn open() -> Result<Wallet> {
    let h = home()?;
    let (cfg, st) = (h.join("wallet_config.json"), h.join("storage.json"));
    let mut w = if st.exists() {
        WalletCore::new_update_chain(cfg, st, None, "").map_err(ef)?
    } else {
        let (w, _) = WalletCore::new_init_storage(cfg, st, None, "").map_err(ef)?;
        w.store_persistent_data().map_err(ef)?;
        w
    };
    if let Ok(id) = std::fs::read_to_string(h.join("msig-fee-account")) {
        w.set_private_fee_payer(Some(id.trim().parse().map_err(ef)?));
    }
    Ok(w)
}

fn vault_path(cli: &Cli) -> Result<PathBuf> {
    Ok(cli.vault.clone().map_or_else(|| home().map(|h| h.join("msig-vault.json")), Ok)?)
}

fn load_vault(cli: &Cli) -> Result<AccountId> {
    let p = vault_path(cli)?;
    let f: VaultFile = serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("no vault file {}: run `msig create` or `msig join`", p.display()))?)?;
    f.v.parse().map_err(ef)
}

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
    ensure!(!block.body.is_reverted(pos), "{what}: tx {hash} REVERTED in block {block_id}");
    if let LeeTransaction::PrivacyPreserving(pp) = &tx {
        ensure!(
            pp.message.execution.fee.as_ref().is_none_or(|f| f.refund_to.is_none()),
            "{what}: refund_to set"
        );
    }
    println!("  {what}: tx {hash} included in block {block_id}");
    Ok(block_id)
}

/// Sync right before the spend; the caller re-reads state after the tx is included.
/// Progress is printed every 5 s while the (long) simulate + prove + submit call runs.
async fn private_tx(w: &mut Wallet, accounts: Vec<AccountMention>, ix: &Instruction, what: &str) -> Result<()> {
    println!("  [sync] wallet to latest block");
    w.sync_to_latest_block().await.map_err(ef)?;
    let header = ProgramHeader::from_bytes(
        w.get_account_view(Actor::new(program_account(), PROGRAM_LOADER_ACCOUNT_ID))
            .await
            .map_err(ef)?
            .ok_or_else(|| anyhow!("no header account at P: run `msig setup` (is the sequencer up?)"))?
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
    let t = Instant::now();
    println!("  [prove] simulating, proving, submitting {what}");
    let programs = catalog_undisclosed(header, proof);
    let hash = {
        let fut = w.send_tx(
            accounts,
            0,
            borsh::to_vec(ix)?,
            &programs,
            None,
            AsyncDelivery::default(),
        );
        tokio::pin!(fut);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.tick().await;
        loop {
            tokio::select! {
                r = &mut fut => break r.map_err(ef)?.0,
                _ = tick.tick() => println!("  [prove] still working, {:.0} s elapsed", t.elapsed().as_secs_f64()),
            }
        }
    };
    println!("  [prove] done in {:.1} s", t.elapsed().as_secs_f64());
    println!("  [wait] for inclusion");
    included(w, hash, what).await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    Ok(())
}

/// `V` as this wallet decrypted it: `(multisig state, native balance)`.
fn vault_state(w: &Wallet, v: AccountId) -> Result<(Option<Multisig>, u128)> {
    let acct = &w
        .storage()
        .key_chain()
        .shared_private_account(v)
        .ok_or_else(|| anyhow!("vault {v} not tracked by this wallet: `msig join` the vault file"))?
        .account;
    let ms = acct.data.actor_state(program_account());
    let ms = (!ms.is_empty()).then(|| borsh::from_slice(ms)).transpose()?;
    Ok((ms, decode_balance(acct.data.actor_state(NATIVE_TOKEN_PROGRAM_ID)).map_err(ef)?))
}

async fn synced_state(w: &mut Wallet, v: AccountId) -> Result<(Multisig, u128)> {
    w.sync_to_latest_block().await.map_err(ef)?;
    let (ms, bal) = vault_state(w, v)?;
    Ok((ms.ok_or_else(|| anyhow!("vault has no multisig state yet (Create not confirmed?)"))?, bal))
}

fn guest_error(e: &anyhow::Error) -> Option<String> {
    let s = format!("{e:#}");
    let code: u32 = s.split("code: ").nth(1)?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    s.contains("ProgramExitedWithCode").then(|| format!("{:?} (guest exit code {code})", msig_core::Error::from_code(code)))
}

fn parse_pub(h: &str) -> Result<[u8; 32]> {
    hex::decode(h)?.try_into().map_err(|_| anyhow!("public key must be 32 bytes of hex"))
}

fn print_status(ms: &Multisig, bal: u128) {
    println!("threshold {} of {}  epoch {}  nonce {}  balance {bal}", ms.threshold, ms.members.len(), ms.config_epoch, ms.nonce);
    let limit = ms.limit.as_ref().map_or("none".into(), |l| format!("{} per {} ms ({} sig)", l.amount, l.period, l.threshold));
    let queued = ms.queued.as_ref().map_or("empty".into(), |q| format!("eta {}", q.t + ms.time_lock));
    println!(
        "time lock {} ms  limit {limit}  spent {} in period {}  spend nonce {}  queue {queued}",
        ms.time_lock, ms.spend.spent, ms.spend.period_index, ms.spend.nonce
    );
    for (i, m) in ms.members.iter().enumerate() {
        println!("  member {i}  {}  perms {:#06b}", hex::encode(m.key), m.perms);
    }
}

fn write_envelope(env: &Envelope, out: &Path) -> Result<()> {
    std::fs::write(out, env.to_json())?;
    println!("wrote {}", out.display());
    Ok(())
}

async fn propose(cli: &Cli, action: Action, p: &Prop) -> Result<()> {
    let sk = read_key(&p.key)?;
    let v = load_vault(cli)?;
    let mut w = open()?;
    let (ms, _) = synced_state(&mut w, v).await?;
    let me = ms.members.iter().find(|m| m.key == pubkey32(&sk));
    ensure!(me.is_some_and(|m| m.perms & INITIATE != 0), "this key is not a member with the initiate permission");
    let mut env = Envelope::new(&v, ms.config_epoch, ms.nonce, now_ms() + p.expiry_secs * 1000, &action);
    env.sign(Role::Propose, &sk)?;
    write_envelope(&env, &p.out)?;
    print!("{}", render(&env, &Registry::load(None)?)?.0);
    Ok(())
}

async fn setup(payer_key: &Path) -> Result<()> {
    let h = home()?;
    std::fs::create_dir_all(&h)?;
    let mut w = open()?;
    let payer_sk = lee::PrivateKey::try_new(read_secret(payer_key)?).map_err(ef)?;
    let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&payer_sk));
    w.storage_mut().key_chain_mut().add_imported_public_account(payer_sk);
    let p = program_account();
    if ProgramLoader(&mut w).program(p).await.map_err(ef)?.is_none() {
        println!("[setup] deploying multisig image {} at P = {p}", msig_host::msig_image_hex());
        w.storage_mut()
            .key_chain_mut()
            .add_imported_public_account(lee::PrivateKey::try_new(deploy_key(MSIG_ID)).map_err(ef)?);
        let user_elf = risc0_binfmt::ProgramBinary::decode(MSIG_ELF).map_err(ef)?.user_elf;
        let segs: Vec<AccountId> = std::iter::repeat_with(|| w.create_new_account_public(None).0)
            .take(user_elf.len().div_ceil(MAX_SEGMENT_DATA_LEN))
            .collect();
        w.store_persistent_data().map_err(ef)?;
        ProgramLoader(&mut w).deploy(p, &segs, MSIG_ELF.to_vec(), true, Some(payer)).await.map_err(ef)?;
    } else {
        println!("[setup] program already deployed at P = {p}");
    }
    if h.join("msig-fee-account").exists() {
        println!("[setup] fee account already present");
        return Ok(());
    }
    let (fee, _) = w.create_new_account_private(None);
    w.store_persistent_data().map_err(ef)?;
    w.sync_to_latest_block().await.map_err(ef)?;
    let (hash, _) = NativeTokenTransfer(&w)
        .transfer(AccountIdentity::Public(payer), AccountIdentity::PrivateOwned(fee), FEE_FUNDS, CreditDelivery::Automatic)
        .await
        .map_err(ef)?;
    included(&w, hash, "shield fee funds").await?;
    w.sync_to_latest_block().await.map_err(ef)?;
    w.store_persistent_data().map_err(ef)?;
    std::fs::write(h.join("msig-fee-account"), fee.to_string())?;
    println!("[setup] private fee account {fee}");
    Ok(())
}

async fn join_vault(w: &mut Wallet, f: &VaultFile, created: bool) -> Result<AccountId> {
    let gms: [u8; 32] = hex::decode(&f.gms)?.try_into().map_err(|_| anyhow!("bad gms"))?;
    let s: [u8; 32] = hex::decode(&f.seed)?.try_into().map_err(|_| anyhow!("bad seed"))?;
    let label = wallet::account::Label::new(GROUP);
    w.insert_group_key_holder(label.clone(), GroupKeyHolder::from_gms(gms), created);
    let info = w
        .create_shared_pda_account_at(label, PdaSeed::new(s), MSIG_ID, program_account())
        .await
        .map_err(ef)?;
    ensure!(info.account_id.to_string() == f.v, "vault file: id {} does not match its group secret + seed", f.v);
    w.store_persistent_data().map_err(ef)?;
    Ok(info.account_id)
}

fn me_vault(p: AccountId, v: AccountId) -> AccountMention {
    AccountIdentity::PrivateOwned(v).select_program_actor_state(p)
}

async fn run(cli: Cli) -> Result<()> {
    let p = program_account();
    match &cli.cmd {
        Cmd::Keygen { out } => {
            let sk = SigningKey::from_bytes(&random32()?).map_err(|e| anyhow!("{e}"))?;
            write_secret(out, &hex::encode(sk.to_bytes()))?;
            println!("{}", hex::encode(pubkey32(&sk)));
        }
        Cmd::Setup { payer_key } => setup(payer_key).await?,
        Cmd::Create { members, threshold, perms, chain_tag } => {
            let cfg = InitConfig {
                chain_tag: Sha256::digest(chain_tag.as_bytes()).into(),
                members: members
                    .iter()
                    .map(|m| Ok(Member { key: parse_pub(m)?, perms: *perms }))
                    .collect::<Result<_>>()?,
                threshold: *threshold,
            };
            msig_core::validate(&Multisig::new(&cfg, [0; 32])).map_err(|e| anyhow!("invalid config: {e:?}"))?;
            let (gms, salt) = (random32()?, random32()?);
            let s = seed(&cfg, &salt);
            let mut w = open()?;
            let (v, keys) = msig_host::private_vault(&GroupKeyHolder::from_gms(gms), s);
            let vf = VaultFile { gms: hex::encode(gms), seed: hex::encode(s), v: v.to_string() };
            let path = vault_path(&cli)?;
            write_secret(&path, &serde_json::to_string_pretty(&vf)?)?; // before Create: the group secret must not be lost
            join_vault(&mut w, &vf, true).await?;
            println!("[create] vault V = {v}; vault file {} (secret: give it to members out of band)", path.display());
            let ix = Instruction::Create {
                cfg,
                salt,
                keys: Some((keys.generate_nullifier_public_key(), keys.generate_viewing_public_key())),
            };
            private_tx(&mut w, vec![me_vault(p, v)], &ix, "Create").await?;
            let (ms, bal) = synced_state(&mut w, v).await?;
            print_status(&ms, bal);
        }
        Cmd::Join { file } => {
            let f: VaultFile = serde_json::from_slice(&std::fs::read(file)?)?;
            let mut w = open()?;
            let v = join_vault(&mut w, &f, false).await?;
            write_secret(&vault_path(&cli)?, &serde_json::to_string_pretty(&f)?)?;
            println!("joined vault {v}");
        }
        Cmd::Fund { amount, payer_key, to_key } => {
            let mut w = open()?;
            let sk = lee::PrivateKey::try_new(read_secret(payer_key)?).map_err(ef)?;
            let payer = AccountId::from(&lee::PublicKey::new_from_private_key(&sk));
            w.storage_mut().key_chain_mut().add_imported_public_account(sk);
            w.sync_to_latest_block().await.map_err(ef)?;
            let (to, what) = match to_key {
                Some(k) => {
                    let pk = lee::PublicKey::new_from_private_key(&lee::PrivateKey::try_new(read_secret(k)?).map_err(ef)?);
                    println!("public account {}", AccountId::from(&pk));
                    (AccountIdentity::PublicForeign(pk), "fund public account")
                }
                None => (AccountIdentity::PrivateOwned(load_vault(&cli)?), "fund vault"),
            };
            let (hash, _) = NativeTokenTransfer(&w)
                .transfer(AccountIdentity::Public(payer), to, *amount, CreditDelivery::Automatic)
                .await
                .map_err(ef)?;
            included(&w, hash, what).await?;
            w.sync_to_latest_block().await.map_err(ef)?;
            w.store_persistent_data().map_err(ef)?;
        }
        Cmd::Status => {
            let v = load_vault(&cli)?;
            let mut w = open()?;
            let (ms, bal) = synced_state(&mut w, v).await?;
            println!("vault {v}");
            print_status(&ms, bal);
        }
        Cmd::Propose { to, amount, p } => {
            let to: AccountId = to.parse().map_err(|e| anyhow!("--to: {e:?}"))?;
            propose(&cli, msig_host::native_transfer(load_vault(&cli)?, to, *amount), p).await?;
        }
        Cmd::Config(c) => match c {
            ConfigCmd::AddMember { pubkey, perms, p } => {
                let m = Member { key: parse_pub(pubkey)?, perms: *perms };
                propose(&cli, Action::Config(ConfigOp::AddMember(m)), p).await?
            }
            ConfigCmd::RemoveMember { pubkey, p } => {
                propose(&cli, Action::Config(ConfigOp::RemoveMember { key: parse_pub(pubkey)? }), p).await?
            }
            ConfigCmd::SetThreshold { threshold, p } => {
                propose(&cli, Action::Config(ConfigOp::SetThreshold(*threshold)), p).await?
            }
            ConfigCmd::SetTimeLock { ms, p } => propose(&cli, Action::Config(ConfigOp::SetTimeLock(*ms)), p).await?,
            ConfigCmd::SetLimit { amount, period_ms, threshold, p } => {
                let l = Limit { amount: *amount, period: *period_ms, threshold: *threshold };
                propose(&cli, Action::Config(ConfigOp::SetLimit(Some(l))), p).await?
            }
        },
        Cmd::Spend { to, amount, key, out } => {
            let to: AccountId = to.parse().map_err(|e| anyhow!("--to: {e:?}"))?;
            let sk = read_key(key)?;
            let v = load_vault(&cli)?;
            let mut w = open()?;
            let (ms, _) = synced_state(&mut w, v).await?;
            let l = ms.limit.as_ref().ok_or_else(|| anyhow!("NO LIMIT: the vault has no spending limit (config set-limit)"))?;
            ensure!(ms.members.iter().any(|m| m.key == pubkey32(&sk) && m.perms & SPEND != 0), "this key is not a member with the spend permission");
            let mut env = Envelope::new_spend(&v, ms.config_epoch, ms.spend.nonce, now_ms() / l.period, &to, *amount);
            env.sign(Role::Spend, &sk)?;
            write_envelope(&env, out)?;
            print!("{}", render(&env, &Registry::load(cli.registry.as_deref())?)?.0);
        }
        Cmd::Show { envelope } => {
            let env = Envelope::from_json(&std::fs::read_to_string(envelope)?)?;
            let (txt, ok) = render(&env, &Registry::load(cli.registry.as_deref())?)?;
            print!("{txt}");
            ensure!(ok, "some signatures do not verify against the signed bytes");
        }
        Cmd::Approve { envelope, key } | Cmd::Reject { envelope, key } => {
            let mut env = Envelope::from_json(&std::fs::read_to_string(envelope)?)?;
            let role = match (&cli.cmd, env.spend) {
                (Cmd::Approve { .. }, true) => Role::Spend,
                (Cmd::Approve { .. }, false) => Role::Approve,
                _ => Role::Reject,
            };
            let sk = read_key(key)?;
            let (txt, _) = render(&env, &Registry::load(cli.registry.as_deref())?)?;
            print!("{txt}"); // you sign what is printed here, decoded from the signed bytes
            env.sign(role, &sk)?;
            write_envelope(&env, envelope)?;
            println!("signed as {} by {}", role.name(), hex::encode(pubkey32(&sk)));
        }
        Cmd::Execute { envelope, key } => execute(&cli, envelope, key, Stage::Execute).await?,
        Cmd::Queue { envelope, key } => execute(&cli, envelope, key, Stage::Queue).await?,
        Cmd::ExecuteQueued { envelope, key } => execute(&cli, envelope, key, Stage::ExecuteQueued).await?,
    }
    Ok(())
}

/// Spend envelope: `>= limit.threshold` valid `/spend` signatures of current spend members.
fn spend_ix(env: &Envelope, ms: &Multisig, v: AccountId) -> Result<Instruction> {
    let d = env.decode_spend()?;
    ensure!(d.vault == v, "envelope is for vault {}, this wallet's vault is {v}", d.vault);
    ensure!(
        d.epoch == ms.config_epoch && d.nonce == ms.spend.nonce,
        "STALE: spend was signed for epoch {} spend nonce {}, the vault is now at epoch {} spend nonce {}",
        d.epoch, d.nonce, ms.config_epoch, ms.spend.nonce
    );
    let l = ms.limit.as_ref().ok_or_else(|| anyhow!("NO LIMIT: the vault has no spending limit"))?;
    ensure!(now_ms() / l.period == d.p, "EXPIRED: spend period {} is not the current one", d.p);
    let mut signers: Vec<Sig> = env
        .verify()?
        .into_iter()
        .filter(|(s, ok)| *ok && s.role == Role::Spend)
        .filter_map(|(s, _)| {
            let i = ms.members.iter().position(|m| m.key == s.key && m.perms & SPEND != 0)?;
            Some(Sig { index: i as u8, sig: s.sig })
        })
        .collect();
    signers.sort_by_key(|s| s.index);
    signers.dedup_by_key(|s| s.index);
    println!("[3/4] {} valid spend signature(s) from current spend members (limit threshold {})", signers.len(), l.threshold);
    ensure!(signers.len() >= usize::from(l.threshold), "BELOW THRESHOLD: need {} spend signatures, have {}", l.threshold, signers.len());
    Ok(Instruction::Spend { to: d.to, amount: d.amount, p: d.p, signers })
}

/// Action envelope at `stage`: quorum for `Execute` / `Queue`, the queued slot for `ExecuteQueued`.
fn action_ix(env: &Envelope, ms: &Multisig, v: AccountId, sk: &SigningKey, stage: Stage) -> Result<Instruction> {
    let d = env.decode()?;
    ensure!(d.vault == v, "envelope is for vault {}, this wallet's vault is {v}", d.vault);
    ensure!(
        d.epoch == ms.config_epoch && d.nonce == ms.nonce,
        "STALE: envelope was signed for epoch {} nonce {}, the vault is now at epoch {} nonce {}. Its approvals are dead: propose again.",
        d.epoch, d.nonce, ms.config_epoch, ms.nonce
    );
    ensure!(now_ms() < d.expiry, "EXPIRED: envelope expiry {} has passed", d.expiry);
    let idx = |k: &[u8; 32], perm: u8| ms.members.iter().position(|m| &m.key == k && m.perms & perm != 0).map(|i| i as u8);
    let sign = |d: &[u8; 32]| sk.sign_raw(d, &[0; 32]).map(|s| s.to_bytes()).map_err(|e| anyhow!("{e}"));
    let me = |perm: u8, what: &str| idx(&pubkey32(sk), perm).ok_or_else(|| anyhow!("--key is not a member with the {what} permission"));
    if stage == Stage::ExecuteQueued {
        let q = ms.queued.as_ref().ok_or_else(|| anyhow!("NOT QUEUED: the vault's queue slot is empty"))?;
        let [_, _, dq, de] = digests(&v, ms, &d.action, d.expiry, q.t);
        ensure!(dq == q.digest, "NOT QUEUED: the queued action is a different one");
        let eta = q.t + ms.time_lock;
        ensure!(now_ms() >= eta, "TOO EARLY: eta {eta} (unix ms), now {}; the time lock has not run out", now_ms());
        println!("[3/4] queued action matches, eta {eta} passed");
        return Ok(Instruction::ExecuteQueued { action: d.action, executor: Sig { index: me(EXECUTE, "execute")?, sig: sign(&de)? } });
    }
    let sigs: Vec<_> = env.verify()?.into_iter().filter(|(_, ok)| *ok).map(|(s, _)| s).collect();
    let proposer = sigs
        .iter()
        .find(|s| s.role == Role::Propose && idx(&s.key, INITIATE).is_some())
        .ok_or_else(|| anyhow!("no valid proposal signature from a current member with the initiate permission"))?;
    let mut approvals: Vec<Sig> = sigs
        .iter()
        .filter(|s| s.role == Role::Approve)
        .filter_map(|s| idx(&s.key, VOTE).map(|index| Sig { index, sig: s.sig }))
        .collect();
    approvals.sort_by_key(|s| s.index);
    approvals.dedup_by_key(|s| s.index);
    let rejects = sigs.iter().filter(|s| s.role == Role::Reject).count();
    println!("[3/4] {} valid approval(s) from current members (threshold {}), {rejects} advisory rejection(s)", approvals.len(), ms.threshold);
    ensure!(approvals.len() >= usize::from(ms.threshold), "BELOW THRESHOLD: need {} approvals, have {}", ms.threshold, approvals.len());
    let proposer = Sig { index: idx(&proposer.key, INITIATE).unwrap(), sig: proposer.sig };
    Ok(if stage == Stage::Queue {
        ensure!(ms.time_lock != 0, "NO TIME LOCK: the vault executes immediately; use execute");
        let t = now_ms() + QUEUE_LEAD_MS;
        ensure!(t + ms.time_lock < d.expiry, "EXPIRED: eta {} is not before the envelope expiry {}", t + ms.time_lock, d.expiry);
        let [_, _, dq, _] = digests(&v, ms, &d.action, d.expiry, t);
        let qi = me(INITIATE, "initiate")?;
        ensure!(qi == proposer.index, "NOT PROPOSER: only the proposer queues (approvers do not sign t)");
        println!("      queue at t = {t}, eta = {} (unix ms)", t + ms.time_lock);
        Instruction::Queue { action: d.action, expiry: d.expiry, t, proposer, approvals, queuer: Sig { index: qi, sig: sign(&dq)? } }
    } else {
        let [.., de] = digests(&v, ms, &d.action, d.expiry, 0);
        Instruction::Execute { action: d.action, expiry: d.expiry, proposer, approvals, executor: Sig { index: me(EXECUTE, "execute")?, sig: sign(&de)? } }
    })
}

async fn execute(cli: &Cli, envelope: &Path, key: &Path, stage: Stage) -> Result<()> {
    let env = Envelope::from_json(&std::fs::read_to_string(envelope)?)?;
    ensure!(!env.spend || stage == Stage::Execute, "a spend envelope is submitted with `execute`");
    let sk = read_key(key)?;
    let v = load_vault(cli)?;
    let mut w = open()?;
    let me = me_vault(program_account(), v);
    let payee = |r| vec![me.clone(), AccountIdentity::PublicNoSign(r).balance().receiving()];
    let accounts = if env.spend {
        payee(env.decode_spend()?.to)
    } else {
        match (&env.decode()?.action, stage) {
            (_, Stage::Queue) => vec![me.clone()],
            (Action::Call { to, data, with_authority }, _) => {
                let ok = to.program_account_id == NATIVE_TOKEN_PROGRAM_ID && to.account_id == v && *with_authority;
                match (ok, borsh::from_slice::<Native>(data)) {
                    (true, Ok(Native::Transfer { to: r, .. })) => payee(r),
                    _ => bail!("prototype CLI executes native transfers out of the vault, config changes and cancel only"),
                }
            }
            _ => vec![me.clone()],
        }
    };
    let what = match (env.spend, stage) {
        (true, _) => "Spend",
        (_, Stage::Execute) => "Execute",
        (_, Stage::Queue) => "Queue",
        (_, Stage::ExecuteQueued) => "ExecuteQueued",
    };
    for attempt in 0..2 {
        println!("[1/4] syncing wallet");
        let (ms, bal) = synced_state(&mut w, v).await?;
        println!("[2/4] vault at epoch {} nonce {}, balance {bal}", ms.config_epoch, ms.nonce);
        let ix = if env.spend { spend_ix(&env, &ms, v)? } else { action_ix(&env, &ms, v, &sk, stage)? };
        println!("[4/4] proving and submitting {what}");
        match private_tx(&mut w, accounts.clone(), &ix, what).await {
            Ok(()) => break,
            Err(e) => {
                if let Some(g) = guest_error(&e) {
                    bail!("the program rejected the {what}: {g}");
                }
                if attempt == 1 {
                    return Err(e.context(format!("{what} failed again after re-syncing")));
                }
                eprintln!("wallet state looked stale ({e:#}); re-syncing and retrying once");
            }
        }
    }
    let (ms, bal) = synced_state(&mut w, v).await?;
    println!("{}. vault now:", if stage == Stage::Queue { "queued" } else { "executed" });
    print_status(&ms, bal);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run(Cli::parse()).await
}
