//! RFP-005 multisig program core (docs/ARCHITECTURE.md §2): state, instructions, the §2.4 digests
//! and the pure handler shared by the guest and the host tests.
//!
//! Prototype scope: `Create`, immediate `Execute` (M-of-N BIP-340 quorum, sequential nonce, roles),
//! `Queue` + `ExecuteQueued` under a time lock (one queued slot), and `Spend` under a per-period
//! native spending limit. Actions: `Call`, `Config` (bumps `config_epoch`), `Cancel`.

use borsh::{BorshDeserialize, BorshSerialize};
use k256::schnorr::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
use lee_core::NullifierPublicKey;
use lee_core::account::{AccountId, Actor};
use lee_core::encryption::ViewingPublicKey;
use lee_core::native_token::custody_transfer;
use lee_core::program::{PdaSeed, ReceiveInput, Response, SyncMessage};
use sha2::{Digest, Sha256};

/// State layout version; v1 states (no time lock, limit, queue or spend) are rejected, not migrated.
pub const VERSION: u8 = 2;
/// A7: worst case `Config` at M = N = (N+2)·~295K-cycle verifications + N key decodes <= 10M gas cap (msig-host `worst_case_quorum_fits_the_block_gas_cap`: 9.58M guest cycles at N = M = 30, ~4% headroom; 9.88M at 31).
pub const MAX_MEMBERS: usize = 30;
/// Upper bound on `time_lock` (30 days in ms, the unit of chain timestamps): a larger lock would
/// make every later Queue impossible while lowering it again must itself be queued.
pub const MAX_TIME_LOCK: u64 = 30 * 24 * 3600 * 1000;

/// Member permission bits (§2.2).
pub const INITIATE: u8 = 1;
pub const VOTE: u8 = 2;
pub const EXECUTE: u8 = 4;
pub const SPEND: u8 = 8;
pub const ALL_PERMS: u8 = INITIATE | VOTE | EXECUTE | SPEND;

pub const TAG_SEED: &str = "LEZ-MSIG/v1/seed";
pub const TAG_PROPOSE: &str = "LEZ-MSIG/v1/propose";
pub const TAG_APPROVE: &str = "LEZ-MSIG/v1/approve";
pub const TAG_EXECUTE: &str = "LEZ-MSIG/v1/execute";
pub const TAG_QUEUE: &str = "LEZ-MSIG/v1/queue";
pub const TAG_SPEND: &str = "LEZ-MSIG/v1/spend";

/// Fixed length of every private note (`ProvingInput.ciphertext_padding`, §3.2): the state at
/// N = MAX_MEMBERS (~1.2 KiB) fits, so note length does not reveal N or the config.
pub const NOTE_PAD: u32 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Member {
    /// BIP-340 x-only public key.
    pub key: [u8; 32],
    pub perms: u8,
}

/// What `V` commits to through its seed (§2.1).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct InitConfig {
    pub chain_tag: [u8; 32],
    pub members: Vec<Member>,
    pub threshold: u8,
}

/// Native spending limit (§2.2): `threshold` SPEND members may move up to `amount` per `period`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Limit {
    pub amount: u128,
    /// Milliseconds (chain timestamps).
    pub period: u64,
    pub threshold: u8,
}

/// The single queued action (prototype limit): `digest` is its `/queue` digest, which binds
/// `config_epoch` and `nonce`; `eta = t + time_lock`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Queued {
    pub digest: [u8; 32],
    pub t: u64,
    pub expiry: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SpendState {
    pub period_index: u64,
    pub spent: u128,
    pub nonce: u64,
}

/// Actor state `(V, P)` (§2.2, prototype subset: no `last`).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Multisig {
    pub version: u8,
    pub chain_tag: [u8; 32],
    pub seed: [u8; 32],
    pub members: Vec<Member>,
    pub threshold: u8,
    /// Milliseconds; 0 = immediate.
    pub time_lock: u64,
    pub limit: Option<Limit>,
    pub config_epoch: u64,
    pub nonce: u64,
    pub queued: Option<Queued>,
    pub spend: SpendState,
}

impl Multisig {
    /// Fresh state at `Create`: epoch 0, nonce 0, no time lock, no limit.
    pub fn new(cfg: &InitConfig, seed: [u8; 32]) -> Self {
        Self {
            version: VERSION,
            chain_tag: cfg.chain_tag,
            seed,
            members: cfg.members.clone(),
            threshold: cfg.threshold,
            time_lock: 0,
            limit: None,
            config_epoch: 0,
            nonce: 0,
            queued: None,
            spend: SpendState::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Action {
    /// `SyncMessage { to, message: data, pda_seeds: {seed} if with_authority }`.
    Call {
        to: Actor,
        data: Vec<u8>,
        with_authority: bool,
    },
    /// One config change (§2.3): same quorum as any action, bumps `config_epoch` (so every
    /// approval signed at the old epoch dies, R.2) and re-checks the §2.2 invariants.
    Config(ConfigOp),
    /// Consumes the nonce and does nothing else: the on-chain way to kill an action.
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum ConfigOp {
    AddMember(Member),
    /// By key, not index: indices shift on removal.
    RemoveMember {
        key: [u8; 32],
    },
    SetThreshold(u8),
    SetTimeLock(u64),
    SetLimit(Option<Limit>),
}

/// A member's BIP-340 signature, by member index.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Sig {
    pub index: u8,
    pub sig: [u8; 64],
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Instruction {
    /// `keys = Some((npk, vpk))` for the private posture, `None` for a public PDA.
    Create {
        cfg: InitConfig,
        salt: [u8; 32],
        keys: Option<(NullifierPublicKey, ViewingPublicKey)>,
    },
    /// Immediate execution (time lock 0): proposer, >= M approvals (strictly ascending index),
    /// executor. Valid while `timestamp < expiry`.
    Execute {
        action: Action,
        expiry: u64,
        proposer: Sig,
        approvals: Vec<Sig>,
        executor: Sig,
    },
    /// Starts the time lock (§2.3): quorum as for `Execute`, plus an `INITIATE` member's `/queue`
    /// signature over `body ‖ t`. Valid while `timestamp < t` (and `>= old.expiry` if the slot is
    /// taken), so `eta = t + time_lock >= now + time_lock`; needs `t + time_lock < expiry`.
    Queue {
        action: Action,
        expiry: u64,
        t: u64,
        proposer: Sig,
        approvals: Vec<Sig>,
        queuer: Sig,
    },
    /// Executes the queued action: an `EXECUTE` member's `/execute` signature over
    /// `body(queued.expiry) ‖ queued.t`. Valid in `[t + time_lock, expiry)`.
    ExecuteQueued { action: Action, executor: Sig },
    /// Native transfer under the limit, signed by `>= limit.threshold` SPEND members (strictly
    /// ascending index) under `/spend`. Valid in `[p·period, (p+1)·period)`.
    Spend {
        to: AccountId,
        amount: u128,
        p: u64,
        signers: Vec<Sig>,
    },
}

/// Program failure; the guest exits with `code as u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Error {
    Malformed = 1,
    AlreadyInitialized = 2,
    Uninitialized = 3,
    WrongAccount = 4,
    InvalidConfig = 5,
    BelowThreshold = 6,
    UnsortedSigners = 7,
    NotMember = 8,
    MissingPermission = 9,
    BadSignature = 10,
    AuthorityOnForeignAccount = 11,
    Reentry = 12,
    Overflow = 13,
    /// State `version` is not `VERSION` (a v1 state is not migrated).
    UnsupportedVersion = 14,
    /// Immediate `Execute` of a non-`Cancel` action under a time lock, or `Queue` without one.
    TimeLockMismatch = 15,
    /// The queue slot holds an entry that has not expired before `t`.
    QueueOccupied = 16,
    /// Nothing queued, or the queued digest does not match (other action, nonce or epoch).
    NotQueued = 17,
    /// Empty validity window (`t + time_lock >= expiry`).
    BadWindow = 18,
    NoLimit = 19,
    OverLimit = 20,
    /// `p` is before the current spend period.
    StalePeriod = 21,
    /// `Queue` by a member other than the proposer (approvers do not sign `t`; only the proposer picks it).
    QueuerNotProposer = 22,
}

impl Error {
    pub fn from_code(code: u32) -> Option<Self> {
        use Error::*;
        [
            Malformed,
            AlreadyInitialized,
            Uninitialized,
            WrongAccount,
            InvalidConfig,
            BelowThreshold,
            UnsortedSigners,
            NotMember,
            MissingPermission,
            BadSignature,
            AuthorityOnForeignAccount,
            Reentry,
            Overflow,
            UnsupportedVersion,
            TimeLockMismatch,
            QueueOccupied,
            NotQueued,
            BadWindow,
            NoLimit,
            OverLimit,
            StalePeriod,
            QueuerNotProposer,
        ]
        .into_iter()
        .find(|e| *e as u32 == code)
    }
}

fn pad32(tag: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..tag.len()].copy_from_slice(tag.as_bytes());
    out
}

/// `seed = sha256(pad32("LEZ-MSIG/v1/seed") ‖ sha256(borsh(cfg)) ‖ salt)`.
pub fn seed(cfg: &InitConfig, salt: &[u8; 32]) -> [u8; 32] {
    let cfg_hash: [u8; 32] = Sha256::digest(borsh::to_vec(cfg).expect("borsh")).into();
    Sha256::new()
        .chain_update(pad32(TAG_SEED))
        .chain_update(cfg_hash)
        .chain_update(salt)
        .finalize()
        .into()
}

/// `body = epoch u64le ‖ nonce u64le ‖ expiry u64le ‖ sha256(borsh(action))` (56 bytes).
pub fn body(epoch: u64, nonce: u64, expiry: u64, action: &Action) -> [u8; 56] {
    let mut b = [0u8; 56];
    b[..8].copy_from_slice(&epoch.to_le_bytes());
    b[8..16].copy_from_slice(&nonce.to_le_bytes());
    b[16..24].copy_from_slice(&expiry.to_le_bytes());
    b[24..].copy_from_slice(&Sha256::digest(borsh::to_vec(action).expect("borsh")));
    b
}

/// `D(tag, body) = sha256(pad32(tag) ‖ V ‖ body)`.
pub fn digest(tag: &str, v: &AccountId, body: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update(pad32(tag))
        .chain_update(v.value())
        .chain_update(body)
        .finalize()
        .into()
}

/// The digests of an action at `ms`'s epoch and nonce: `(propose, approve, queue, execute)`; the
/// last two are over `body ‖ t u64le` (`t = 0` for an immediate `Execute`).
pub fn digests(
    v: &AccountId,
    ms: &Multisig,
    action: &Action,
    expiry: u64,
    t: u64,
) -> [[u8; 32]; 4] {
    let b = body(ms.config_epoch, ms.nonce, expiry, action);
    let mut bt = [0u8; 64];
    bt[..56].copy_from_slice(&b);
    bt[56..].copy_from_slice(&t.to_le_bytes());
    [
        digest(TAG_PROPOSE, v, &b),
        digest(TAG_APPROVE, v, &b),
        digest(TAG_QUEUE, v, &bt),
        digest(TAG_EXECUTE, v, &bt),
    ]
}

/// The three digests an immediate `Execute` verifies: `(propose, approve, execute)` with `t = 0`.
pub fn execute_digests(
    v: &AccountId,
    ms: &Multisig,
    action: &Action,
    expiry: u64,
) -> [[u8; 32]; 3] {
    let [dp, da, _, de] = digests(v, ms, action, expiry, 0);
    [dp, da, de]
}

/// `D("/spend", epoch ‖ spend.nonce ‖ p u64le ‖ to ‖ amount u128le)` (72-byte body).
pub fn spend_digest(v: &AccountId, ms: &Multisig, to: &AccountId, amount: u128, p: u64) -> [u8; 32] {
    let mut b = [0u8; 72];
    b[..8].copy_from_slice(&ms.config_epoch.to_le_bytes());
    b[8..16].copy_from_slice(&ms.spend.nonce.to_le_bytes());
    b[16..24].copy_from_slice(&p.to_le_bytes());
    b[24..56].copy_from_slice(to.value());
    b[56..].copy_from_slice(&amount.to_le_bytes());
    digest(TAG_SPEND, v, &b)
}

/// §2.2 invariants, checked at `Create` and after every config op.
pub fn validate(ms: &Multisig) -> Result<(), Error> {
    let members = &ms.members;
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return Err(Error::InvalidConfig);
    }
    for (i, m) in members.iter().enumerate() {
        if m.perms == 0
            || m.perms & !ALL_PERMS != 0
            || VerifyingKey::from_bytes(&m.key).is_err()
            || members[..i].iter().any(|o| o.key == m.key)
        {
            return Err(Error::InvalidConfig);
        }
    }
    let count = |p: u8| members.iter().filter(|m| m.perms & p != 0).count();
    if ms.threshold == 0
        || usize::from(ms.threshold) > count(VOTE)
        || count(INITIATE) == 0
        || count(EXECUTE) == 0
        || ms.time_lock > MAX_TIME_LOCK
    {
        return Err(Error::InvalidConfig);
    }
    if let Some(l) = &ms.limit
        && (l.threshold == 0 || usize::from(l.threshold) > count(SPEND) || l.period == 0)
    {
        return Err(Error::InvalidConfig);
    }
    Ok(())
}

fn verify(ms: &Multisig, s: &Sig, perm: u8, digest: &[u8; 32]) -> Result<(), Error> {
    let m = ms
        .members
        .get(usize::from(s.index))
        .ok_or(Error::NotMember)?;
    if m.perms & perm == 0 {
        return Err(Error::MissingPermission);
    }
    let key = VerifyingKey::from_bytes(&m.key).map_err(|_| Error::InvalidConfig)?;
    let sig = Signature::try_from(s.sig.as_slice()).map_err(|_| Error::BadSignature)?;
    key.verify_prehash(digest, &sig)
        .map_err(|_| Error::BadSignature)
}

/// `>= min` signatures under `perm` in strictly ascending index order (so distinct members).
/// Checks order and count before any signature (cheap rejections first).
fn verify_set(ms: &Multisig, sigs: &[Sig], min: u8, perm: u8, d: &[u8; 32]) -> Result<(), Error> {
    if sigs.windows(2).any(|w| w[0].index >= w[1].index) {
        return Err(Error::UnsortedSigners);
    }
    if sigs.len() < usize::from(min) {
        return Err(Error::BelowThreshold);
    }
    sigs.iter().try_for_each(|s| verify(ms, s, perm, d))
}

/// §2.5: proposer (`INITIATE`) under `/propose`, >= M distinct `VOTE` members under `/approve`,
/// and the stage signature (`EXECUTE` under `/execute` for an immediate `Execute` with `t = 0`;
/// `INITIATE` under `/queue` for `Queue`). Digests are recomputed from state and the submitted
/// action, never taken from the caller. M+2 verifications. Returns the `/queue` digest.
#[allow(clippy::too_many_arguments)]
pub fn check_quorum(
    v: &AccountId,
    ms: &Multisig,
    action: &Action,
    expiry: u64,
    t: u64,
    proposer: &Sig,
    approvals: &[Sig],
    stage: (&Sig, u8),
) -> Result<[u8; 32], Error> {
    let [dp, da, dq, de] = digests(v, ms, action, expiry, t);
    verify_set(ms, approvals, ms.threshold, VOTE, &da)?;
    verify(ms, proposer, INITIATE, &dp)?;
    let (sig, perm) = stage;
    verify(ms, sig, perm, if perm == EXECUTE { &de } else { &dq })?;
    Ok(dq)
}

fn load(pre: &[u8]) -> Result<Multisig, Error> {
    match pre.first() {
        None => Err(Error::Uninitialized),
        Some(&VERSION) => borsh::from_slice(pre).map_err(|_| Error::Malformed),
        Some(_) => Err(Error::UnsupportedVersion),
    }
}

fn store(ms: &Multisig) -> Response {
    Response::set_state(borsh::to_vec(ms).expect("borsh"))
}

fn with_call(r: Response, call: Option<SyncMessage>) -> Response {
    match call {
        Some(c) => r.send(c),
        None => r,
    }
}

/// Runs an approved action at the current nonce: `nonce += 1`, the queue slot is cleared, a
/// `Config` is applied (epoch bump, invariants re-checked). Returns the call to dispatch.
fn apply(ms: &mut Multisig, p: &AccountId, v: &AccountId, action: Action) -> Result<Option<SyncMessage>, Error> {
    ms.nonce = ms.nonce.checked_add(1).ok_or(Error::Overflow)?;
    ms.queued = None;
    Ok(match action {
        Action::Call {
            to,
            data,
            with_authority,
        } => {
            if to.program_account_id == *p {
                return Err(Error::Reentry);
            }
            if with_authority && to.account_id != *v {
                return Err(Error::AuthorityOnForeignAccount);
            }
            let call = SyncMessage {
                to,
                message: data,
                pda_seeds: Default::default(),
            };
            Some(if with_authority {
                call.with_pda_seeds([PdaSeed::new(ms.seed)])
            } else {
                call
            })
        }
        Action::Config(op) => {
            match op {
                ConfigOp::AddMember(m) => ms.members.push(m),
                ConfigOp::RemoveMember { key } => {
                    let i = ms
                        .members
                        .iter()
                        .position(|m| m.key == key)
                        .ok_or(Error::NotMember)?;
                    ms.members.remove(i);
                }
                ConfigOp::SetThreshold(t) => ms.threshold = t,
                ConfigOp::SetTimeLock(t) => ms.time_lock = t,
                ConfigOp::SetLimit(l) => {
                    // the counter is in units of the old period; the spend nonce stays (replay guard)
                    ms.limit = l;
                    (ms.spend.period_index, ms.spend.spent) = (0, 0);
                }
            }
            validate(ms)?;
            ms.config_epoch = ms.config_epoch.checked_add(1).ok_or(Error::Overflow)?;
            None
        }
        Action::Cancel => None,
    })
}

/// The program: root messages to `Actor(V, P)`. Replies (`from != None`) keep state (§2.3).
pub fn handle(input: &ReceiveInput) -> Result<Response, Error> {
    if input.from.is_some() {
        return Ok(Response::keep_state());
    }
    let v = input.receiver.account_id;
    let p = input.receiver.program_account_id;
    let ix = borsh::from_slice::<Instruction>(&input.message).map_err(|_| Error::Malformed)?;
    if let Instruction::Create { cfg, salt, keys } = ix {
        if !input.pre_state.is_empty() {
            return Err(Error::AlreadyInitialized);
        }
        let seed = seed(&cfg, &salt);
        let ps = PdaSeed::new(seed);
        let expect = match &keys {
            Some((npk, vpk)) => AccountId::for_private_pda(&p, &ps, npk, vpk),
            None => AccountId::for_public_pda(&p, &ps),
        };
        if expect != v {
            return Err(Error::WrongAccount);
        }
        let ms = Multisig::new(&cfg, seed);
        validate(&ms)?;
        return Ok(store(&ms));
    }
    let mut ms = load(&input.pre_state)?;
    match ix {
        Instruction::Create { .. } => unreachable!(),
        Instruction::Execute {
            action,
            expiry,
            proposer,
            approvals,
            executor,
        } => {
            if ms.time_lock != 0 && action != Action::Cancel {
                return Err(Error::TimeLockMismatch);
            }
            check_quorum(&v, &ms, &action, expiry, 0, &proposer, &approvals, (&executor, EXECUTE))?;
            let call = apply(&mut ms, &p, &v, action)?;
            Ok(with_call(store(&ms), call).timestamp_window(..expiry))
        }
        Instruction::Queue {
            action,
            expiry,
            t,
            proposer,
            approvals,
            queuer,
        } => {
            if ms.time_lock == 0 {
                return Err(Error::TimeLockMismatch);
            }
            if queuer.index != proposer.index {
                return Err(Error::QueuerNotProposer);
            }
            let eta = t.checked_add(ms.time_lock).ok_or(Error::Overflow)?;
            if eta >= expiry {
                return Err(Error::BadWindow);
            }
            let from = match &ms.queued {
                Some(q) if q.expiry >= t => return Err(Error::QueueOccupied),
                Some(q) => Some(q.expiry),
                None => None,
            };
            let dq = check_quorum(&v, &ms, &action, expiry, t, &proposer, &approvals, (&queuer, INITIATE))?;
            ms.queued = Some(Queued { digest: dq, t, expiry });
            store(&ms)
                .try_timestamp_window((from, Some(t)))
                .map_err(|_| Error::BadWindow)
        }
        Instruction::ExecuteQueued { action, executor } => {
            let q = ms.queued.clone().ok_or(Error::NotQueued)?;
            let [_, _, dq, de] = digests(&v, &ms, &action, q.expiry, q.t);
            if dq != q.digest {
                return Err(Error::NotQueued);
            }
            verify(&ms, &executor, EXECUTE, &de)?;
            // the slot only holds entries with t + time_lock < expiry at the time lock they were
            // queued under; a time lock change bumps the nonce, so it is the same time lock here
            let eta = q.t.checked_add(ms.time_lock).ok_or(Error::Overflow)?;
            let call = apply(&mut ms, &p, &v, action)?;
            with_call(store(&ms), call)
                .try_timestamp_window(eta..q.expiry)
                .map_err(|_| Error::BadWindow)
        }
        Instruction::Spend {
            to,
            amount,
            p: period,
            signers,
        } => {
            let limit = ms.limit.clone().ok_or(Error::NoLimit)?;
            verify_set(&ms, &signers, limit.threshold, SPEND, &spend_digest(&v, &ms, &to, amount, period))?;
            if period < ms.spend.period_index {
                return Err(Error::StalePeriod);
            }
            if period > ms.spend.period_index {
                (ms.spend.period_index, ms.spend.spent) = (period, 0);
            }
            ms.spend.spent = ms.spend.spent.checked_add(amount).ok_or(Error::Overflow)?;
            if ms.spend.spent > limit.amount {
                return Err(Error::OverLimit);
            }
            ms.spend.nonce = ms.spend.nonce.checked_add(1).ok_or(Error::Overflow)?;
            let start = period.checked_mul(limit.period).ok_or(Error::Overflow)?;
            let end = start.checked_add(limit.period).ok_or(Error::Overflow)?;
            store(&ms)
                .send(custody_transfer(v, PdaSeed::new(ms.seed), to, amount))
                .try_timestamp_window(start..end)
                .map_err(|_| Error::BadWindow)
        }
    }
}
