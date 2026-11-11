//! Executor-level tests (LEZ `V03State`, real guest, RISC0_DEV_MODE proofs): the private PDA `V`
//! bound to the deployed program account `P`, Create and Execute through the privacy circuit,
//! `gms_holder_cannot_bypass_quorum` (a), (b), (d) and (e) (liveness veto, a known limitation),
//! Config/R.2 cutoff, S.7, `private_tx_byte_scan` and `viewing_key_alone_decrypts_state`.

use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use lee_core::account::{Account, Nonce};
use lee::privacy_preserving_transaction::circuit::ProgramCatalog;
use lee::program::Program;
use lee_core::native_token::Message as Native;
use lee_core::{EncryptionScheme, SharedSecretKey};
use msig_core::{Action, ConfigOp, Error, Multisig, NOTE_PAD, Sig, seed};
use msig_host::*;

const TO: AccountId = AccountId::new([0x77; 32]);
const SALT: [u8; 32] = [5; 32];

fn dev_mode() {
    // SAFETY: set before any thread reads it; every test sets the same value.
    unsafe { std::env::set_var("RISC0_DEV_MODE", "1") };
}

struct Fixture {
    chain: Chain,
    v: AccountId,
    keys: key_protocol::key_management::secret_holders::PrivateKeyHolder,
    seed: [u8; 32],
    ms: Multisig,
    pre: Account,
}

/// A funded private vault (N=3, M=2) at `P`, seeded directly.
fn funded() -> Fixture {
    funded_with(|_| ())
}

/// `funded` with `tweak` applied to the seeded state (test-only: a time lock or limit as if set
/// by an earlier Config).
fn funded_with(tweak: impl FnOnce(&mut Multisig)) -> Fixture {
    dev_mode();
    let cfg = dev_cfg(3, 2);
    let s = seed(&cfg, &SALT);
    let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
    let mut ms = initial_state(&cfg, &SALT);
    tweak(&mut ms);
    let mut chain = Chain::default();
    let pre = chain.seed_vault(v, &keys, &ms, 1_000);
    chain.open_public(TO, 0);
    Fixture {
        chain,
        v,
        keys,
        seed: s,
        ms,
        pre,
    }
}

type TxResult = Result<lee::PrivacyPreservingTransaction, lee::error::LeeError>;

impl Fixture {
    fn execute(&self, approvers: &[u8]) -> TxResult {
        self.run(
            signed_execute(
                self.v,
                &self.ms,
                native_transfer(self.v, TO, 300),
                u64::MAX,
                approvers,
            ),
        )
    }

    /// Proves `ix` as a root message to `Actor(V, P)` against the current `pre`.
    fn run(&self, ix: msig_core::Instruction) -> TxResult {
        let w = self
            .chain
            .witness(self.v, &self.keys, self.seed, Some(self.pre.clone()));
        self.chain
            .prove(
                w,
                Actor::new(self.v, program_account()),
                &ix,
                vec![Actor::native_balance(TO)],
            )
            .map(|(tx, _)| tx)
    }

    /// Settles `tx` and moves the fixture to `V`'s next account with `ms` and `balance`.
    fn advance(&mut self, tx: &lee::PrivacyPreservingTransaction, ms: Multisig, balance: u128) {
        self.chain.settle(tx).unwrap();
        let post = self.next(&ms, balance);
        assert!(self.chain.has(self.v, &post), "V: expected post-state");
        (self.pre, self.ms) = (post, ms);
    }

    /// `V`'s next account: nonce advanced, `ms` and `balance`.
    fn next(&self, ms: &Multisig, balance: u128) -> Account {
        Account {
            nonce: self
                .pre
                .nonce
                .private_account_nonce_increment(&self.keys.nullifier_secret_key()),
            ..Account::funded(balance)
                .with_actor_state(program_account(), borsh::to_vec(ms).unwrap().into())
        }
    }
}

fn member_key(i: u8) -> [u8; 32] {
    dev_key(i).verifying_key().to_bytes().into()
}

#[test]
fn private_vault_is_bound_to_the_deployed_program_account() {
    assert_ne!(program_account(), AccountId::from_builtin_program(MSIG_ID));
}

#[test]
fn create_private_multisig_at_deployed_p() {
    dev_mode();
    let cfg = dev_cfg(3, 2);
    let s = seed(&cfg, &SALT);
    let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
    let mut chain = Chain::default();
    let w = chain.witness(v, &keys, s, None);
    let (tx, _) = chain
        .prove(
            w,
            Actor::new(v, program_account()),
            &create_private(cfg.clone(), SALT, &keys),
            vec![],
        )
        .unwrap();
    chain.settle(&tx).unwrap();
    let expected = Account {
        nonce: Nonce::default().private_account_nonce_increment(&keys.nullifier_secret_key()),
        ..Account::default().with_actor_state(
            program_account(),
            borsh::to_vec(&initial_state(&cfg, &SALT)).unwrap().into(),
        )
    };
    assert!(
        chain.has(v, &expected),
        "post-Create commitment of V is in the commitment set"
    );
}

#[test]
fn create_with_wrong_keys_is_rejected() {
    // the program recomputes V from (P, seed, npk, vpk): keys of another GMS do not match
    dev_mode();
    let cfg = dev_cfg(3, 2);
    let s = seed(&cfg, &SALT);
    let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
    let (_, other) = private_vault(&GroupKeyHolder::new(), s);
    let chain = Chain::default();
    let w = chain.witness(v, &keys, s, None);
    let e = chain
        .prove(
            w,
            Actor::new(v, program_account()),
            &create_private(cfg, SALT, &other),
            vec![],
        )
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::WrongAccount), "{e}");
}

#[test]
fn quorum_execute_moves_native_balance_out_of_v() {
    let mut f = funded();
    let tx = f.execute(&[1, 2]).unwrap();
    f.chain.settle(&tx).unwrap();
    assert_eq!(f.chain.public_balance(TO), 300);
    let post = Account {
        nonce: f
            .pre
            .nonce
            .private_account_nonce_increment(&f.keys.nullifier_secret_key()),
        ..Account::funded(700).with_actor_state(
            program_account(),
            borsh::to_vec(&Multisig {
                nonce: 1,
                ..f.ms.clone()
            })
            .unwrap()
            .into(),
        )
    };
    assert!(f.chain.has(f.v, &post), "V: balance 700, nonce 1");
}

/// The exact LEZ error kind of a native debit of `v` with no seed grant.
fn assert_unauthorized_debit(e: &lee::error::LeeError, v: AccountId, what: &str) {
    use lee::error::{InvalidProgramBehaviorError as B, LeeError};
    use lee_core::native_token::TransferError;
    assert!(
        matches!(
            e,
            LeeError::InvalidProgramBehavior(B::NativeTransferFailed(
                TransferError::UnauthorizedSender { account_id }
            )) if *account_id == v
        ),
        "{what} {e:?}"
    );
}

#[test]
fn gms_holder_cannot_bypass_quorum() {
    let f = funded();
    // (a) the GMS holder sends a root Transfer to Actor(V, native): a PDA witness is never
    // key-authorized, so the native program rejects the debit
    let w = f.chain.witness(f.v, &f.keys, f.seed, Some(f.pre.clone()));
    let e = f
        .chain
        .prove_raw(
            w,
            Actor::native_balance(f.v),
            borsh::to_vec(&Native::Transfer {
                to: TO,
                amount: 300,
            })
            .unwrap(),
            vec![Actor::native_balance(TO)],
        )
        .unwrap_err();
    assert_unauthorized_debit(&e, f.v, "(a)");
    // (a') a full quorum whose Call omits `with_authority`: the multisig sends no seed grant, so
    // the native debit of V is unauthorized, exactly as for the direct debit
    let Action::Call { to, data, .. } = native_transfer(f.v, TO, 300) else {
        unreachable!()
    };
    let no_grant = Action::Call {
        to,
        data,
        with_authority: false,
    };
    let e = f
        .run(signed_execute(f.v, &f.ms, no_grant, u64::MAX, &[1, 2]))
        .unwrap_err();
    assert_unauthorized_debit(&e, f.v, "(a')");
    // (b) Execute with M-1 approvals
    let e = f.execute(&[1]).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BelowThreshold), "(b) {e}");
    // (d) Create on the initialised V
    let w = f.chain.witness(f.v, &f.keys, f.seed, Some(f.pre.clone()));
    let e = f
        .chain
        .prove(
            w,
            Actor::new(f.v, program_account()),
            &create_private(dev_cfg(3, 2), SALT, &f.keys),
            vec![],
        )
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::AlreadyInitialized), "(d) {e}");
    // (c) foreign program with V's seed: not built here (no second program); covered by LEZ's
    // `caller_pda_seeds_with_wrong_seed_rejects_private_pda_for_callee` and the issuer check in
    // `private_seed_grant` (execution_state.rs).
}

#[test]
fn replayed_execute_is_rejected_after_nonce_bump() {
    let mut f = funded();
    let ix = signed_execute(
        f.v,
        &f.ms,
        native_transfer(f.v, TO, 300),
        u64::MAX,
        &[1, 2],
    );
    let tx = f.run(ix.clone()).unwrap();
    f.chain.settle(&tx).unwrap();
    // the identical tx is dead by its spent nullifier (LEZ, not the program)
    assert!(f.chain.settle(&tx).is_err());
    // the program's own nonce check: the same Execute proven against V's post-state fails
    let ms1 = Multisig {
        nonce: 1,
        ..f.ms.clone()
    };
    f.pre = f.next(&ms1, 700);
    f.ms = ms1;
    let e = f.run(ix).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "{e}");
}

#[test]
fn config_removes_member_and_stale_approvals_are_rejected() {
    // R.2 through the privacy circuit: Config(RemoveMember 1) lands, then approvals signed at the
    // old epoch (pre-signed for nonce 1) and the removed member's fresh approval both fail
    let mut f = funded();
    let presigned = signed_execute(
        f.v,
        &Multisig {
            nonce: 1,
            ..f.ms.clone()
        },
        native_transfer(f.v, TO, 300),
        u64::MAX,
        &[1, 2],
    );
    let remove = Action::Config(ConfigOp::RemoveMember { key: member_key(1) });
    let tx = f
        .run(signed_execute(f.v, &f.ms, remove, u64::MAX, &[1, 2]))
        .unwrap();
    f.chain.settle(&tx).unwrap();
    let mut ms1 = f.ms.clone();
    ms1.members.remove(1);
    (ms1.config_epoch, ms1.nonce) = (1, 1);
    let post = f.next(&ms1, 1_000);
    assert!(f.chain.has(f.v, &post), "V: member 1 removed, epoch 1, nonce 1");
    (f.pre, f.ms) = (post, ms1);

    let e = f.run(presigned).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "pre-signed {e}");
    // removed member 1 signs at epoch 1 under its old index (now member 2's key)
    let e = f.execute(&[0, 1]).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "removed {e}");
    // remaining members 0 and old 2 (index 1 now): fresh approvals at epoch 1 pass
    let msig_core::Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = signed_execute(
        f.v,
        &f.ms,
        native_transfer(f.v, TO, 300),
        u64::MAX,
        &[0, 1],
    )
    else {
        unreachable!()
    };
    let [_, da, _] = msig_core::execute_digests(&f.v, &f.ms, &action, expiry);
    approvals[1] = Sig {
        index: 1,
        sig: dev_key(2).sign_raw(&da, &[0; 32]).unwrap().to_bytes(),
    };
    let tx = f
        .run(msig_core::Instruction::Execute {
            action,
            expiry,
            proposer,
            approvals,
            executor,
        })
        .unwrap();
    f.chain.settle(&tx).unwrap();
    assert_eq!(f.chain.public_balance(TO), 300);
}

#[test]
fn approvals_are_attributed_to_the_member_at_their_index() {
    // R.1: member 2's signature presented under member 1's index does not count
    let f = funded();
    let msig_core::Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = signed_execute(
        f.v,
        &f.ms,
        native_transfer(f.v, TO, 300),
        u64::MAX,
        &[1, 2],
    )
    else {
        unreachable!()
    };
    approvals[0].sig = approvals[1].sig;
    let e = f
        .run(msig_core::Instruction::Execute {
            action,
            expiry,
            proposer,
            approvals,
            executor,
        })
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "{e}");
}

#[test]
fn uninitialised_vault_cannot_be_drained() {
    // S.7: V exists only as an address (deposits would be sealed messages); every Execute fails
    dev_mode();
    let cfg = dev_cfg(3, 2);
    let s = seed(&cfg, &SALT);
    let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
    let mut chain = Chain::default();
    chain.open_public(TO, 0);
    let ix = signed_execute(
        v,
        &initial_state(&cfg, &SALT),
        native_transfer(v, TO, 300),
        u64::MAX,
        &[1, 2],
    );
    let w = chain.witness(v, &keys, s, None);
    let e = chain
        .prove(
            w,
            Actor::new(v, program_account()),
            &ix,
            vec![Actor::native_balance(TO)],
        )
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::Uninitialized), "{e}");
}

/// Known limitation (ARCHITECTURE §3.1, risk 1, Q12): any GMS holder, including a removed member,
/// holds `nsk` and can land a transaction that touches `V` without any multisig instruction. It
/// spends `V`'s nullifier, so an honest in-flight `Execute` proven against the old commitment is
/// rejected. Balance and config are unchanged. This test PASSES when the veto is reproduced.
#[test]
fn gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced() {
    let mut f = funded();
    let x = AccountId::new([0x58; 32]); // trivial program X: keeps its state
    let state = std::mem::take(&mut f.chain.state);
    f.chain.state = state.with_named_programs([(x, Program::new(NOOP_ELF.into()).unwrap())]);
    let programs = ProgramCatalog::from([
        (program_account(), Program::new(MSIG_ELF.into()).unwrap()),
        (x, Program::new(NOOP_ELF.into()).unwrap()),
    ]);

    let honest = f.execute(&[1, 2]).unwrap(); // in flight, proven against the current commitment
    let w = f.chain.witness(f.v, &f.keys, f.seed, Some(f.pre.clone()));
    let (veto, _) = f
        .chain
        .prove_tx(vec![w], Actor::new(f.v, x), vec![], vec![], None, &programs)
        .expect("the GMS holder's root message to Actor(V, X) proves");
    f.chain.settle(&veto).expect("the veto lands");

    let e = f.chain.settle(&honest).unwrap_err();
    assert!(
        format!("{e:?}").to_lowercase().contains("nullifier"),
        "honest Execute rejected for a spent nullifier: {e:?}"
    );
    let unchanged = Account {
        nonce: f
            .pre
            .nonce
            .private_account_nonce_increment(&f.keys.nullifier_secret_key()),
        ..f.pre.clone()
    };
    assert!(
        f.chain.has(f.v, &unchanged),
        "V: balance and config unchanged"
    );
    assert_eq!(f.chain.public_balance(TO), 0);
}

#[test]
fn private_tx_byte_scan() {
    // Pr.1: a private Execute with a private fee payer publishes no member key, member account id,
    // fee payer id or V; refund_to is None; every note is NOTE_PAD bytes for N = 3 and N = MAX_MEMBERS
    dev_mode();
    let mut note_lens = Vec::new();
    for n in [3u8, msig_core::MAX_MEMBERS as u8] {
        let cfg = dev_cfg(n, 2);
        let s = seed(&cfg, &SALT);
        let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
        let ms = initial_state(&cfg, &SALT);
        let mut chain = Chain::default();
        let pre = chain.seed_vault(v, &keys, &ms, 1_000);
        chain.open_public(TO, 0);
        let fp = chain.seed_fee_payer(1_000_000);
        let ix = signed_execute(v, &ms, native_transfer(v, TO, 300), u64::MAX, &[1, 2]);
        let (tx, _) = chain
            .prove_tx(
                vec![
                    chain.witness(v, &keys, s, Some(pre)),
                    chain.fee_witness(&fp),
                ],
                Actor::new(v, program_account()),
                borsh::to_vec(&ix).unwrap(),
                vec![Actor::native_balance(TO), Actor::native_balance(fp.inbox)],
                Some(fp.transfer(1_000)),
                &chain.msig_catalog(),
            )
            .unwrap();
        let bytes = borsh::to_vec(&tx).unwrap();
        let mut needles: Vec<(String, [u8; 32])> = vec![
            ("V".into(), *v.value()),
            ("fee payer".into(), *fp.id().value()),
            ("P".into(), *program_account().value()),
            (
                "image id".into(),
                std::array::from_fn(|i| MSIG_ID[i / 4].to_le_bytes()[i % 4]),
            ),
        ];
        for (i, m) in ms.members.iter().enumerate() {
            needles.push((format!("member {i} key"), m.key));
            let id = AccountId::from(&lee::PublicKey::try_new(m.key).unwrap());
            needles.push((format!("member {i} account"), *id.value()));
        }
        // every public field of the submitted tx (message: TransactionMessage<ProvenExecution>,
        // witness set), enumerated from the LEZ types; evidence/leak-surface.md lists the verdicts.
        let (m, e) = (&tx.message, &tx.message.execution);
        let b = |x: &dyn Fn() -> Vec<u8>| x();
        let mut table: Vec<(String, Vec<u8>)> = vec![
            ("public_context.actors".into(), b(&|| borsh::to_vec(&m.public_context.actors).unwrap())),
            ("public_context.authorized_accounts".into(), borsh::to_vec(&m.public_context.authorized_accounts).unwrap()),
            ("public_context.promotions".into(), borsh::to_vec(&m.public_context.promotions).unwrap()),
            ("nonces".into(), borsh::to_vec(&m.nonces).unwrap()),
            ("admission_evidence".into(), borsh::to_vec(&m.admission_evidence).unwrap()),
            ("execution.boundary".into(), borsh::to_vec(&e.boundary).unwrap()),
            ("execution.async_messages".into(), borsh::to_vec(&e.async_messages).unwrap()),
            ("execution.recovery_bindings".into(), borsh::to_vec(&e.recovery_bindings).unwrap()),
            ("execution.public_root".into(), borsh::to_vec(&e.public_root).unwrap()),
            ("execution.validity".into(), borsh::to_vec(&e.validity).unwrap()),
            ("execution.program_image_claims".into(), borsh::to_vec(&e.program_image_claims).unwrap()),
            ("execution.fee".into(), borsh::to_vec(&e.fee).unwrap()),
            ("witness_set".into(), borsh::to_vec(&tx.witness_set).unwrap()),
        ];
        for (i, a) in e.private_actions.iter().enumerate() {
            let k = format!("execution.private_actions[{i}]");
            table.push((format!("{k}.nullifier"), borsh::to_vec(&a.nullifier).unwrap()));
            table.push((format!("{k}.root"), borsh::to_vec(&a.root).unwrap()));
            table.push((format!("{k}.commitment"), borsh::to_vec(&a.commitment).unwrap()));
            table.push((format!("{k}.encrypted_post_state.epk"), borsh::to_vec(&a.encrypted_post_state.epk).unwrap()));
            table.push((format!("{k}.encrypted_post_state.ciphertext"), borsh::to_vec(&a.encrypted_post_state.ciphertext).unwrap()));
        }
        for (field, raw) in &table {
            for (what, needle) in &needles {
                assert!(
                    !raw.windows(32).any(|w| w == needle),
                    "N={n}: {what} in {field}"
                );
            }
            println!("N={n} field {field}: {} bytes", raw.len());
        }
        for (what, needle) in &needles {
            assert!(!bytes.windows(32).any(|w| w == needle), "N={n}: {what} in plaintext");
        }
        // structure: nothing optional is populated beyond the two public recipients and the fee
        let actors: Vec<_> = m.public_context.actors.iter().map(|a| a.account_id).collect();
        assert!(actors.iter().all(|a| *a == TO || *a == fp.inbox), "N={n}: public actors {actors:?}");
        assert!(m.public_context.authorized_accounts.is_empty() && m.public_context.promotions.is_empty());
        assert!(m.nonces.is_empty() && m.admission_evidence.is_empty(), "N={n}: no public signer");
        assert!(e.async_messages.is_empty() && e.recovery_bindings.is_empty());
        assert!(e.public_root.is_none(), "N={n}: private root call");
        assert_eq!(&borsh::to_vec(&tx.witness_set).unwrap()[..4], &[0; 4], "N={n}: no public signatures");
        // the multisig program is claimed Undisclosed (a root only), never by id or image
        assert!(
            matches!(e.program_image_claims.as_slice(), [lee_core::ProgramImageClaim::Undisclosed { .. }]),
            "N={n}: {:?}",
            e.program_image_claims
        );
        let fee = e.fee.as_ref().expect("fee transfer");
        assert_eq!(fee.refund_to, None, "N={n}: refund_to");
        for a in e.private_actions.iter().chain(fee.fallback.iter().map(|f| &f.action)) {
            // borsh Vec<u8>: u32 length prefix, then the bytes (the field is crate-private)
            note_lens.push(borsh::to_vec(&a.encrypted_post_state.ciphertext).unwrap().len() - 4);
        }
        chain.settle(&tx).unwrap();
        assert_eq!(chain.public_balance(TO), 300);
        println!("N={n}: tx {} bytes, notes {note_lens:?}", bytes.len());
    }
    assert_eq!(note_lens.len(), 6, "V, fee payer and the fee fallback note, two runs");
    assert!(
        note_lens.iter().all(|&l| l == NOTE_PAD as usize),
        "{note_lens:?}"
    );
}

#[test]
fn viewing_key_alone_decrypts_state() {
    // F.10 selective disclosure: V's viewing secret key (d, z) alone, no nsk and no GMS, opens
    // V's post-state note: config, nonce and balance
    let f = funded();
    let tx = f.execute(&[1, 2]).unwrap();
    let (d, z) = (
        f.keys.viewing_secret_key.d,
        f.keys.viewing_secret_key.z,
    );
    let opened: Vec<Account> = tx
        .message
        .execution
        .private_actions
        .iter()
        .filter_map(|a| {
            let ss = SharedSecretKey::decapsulate(&a.encrypted_post_state.epk, &d, &z)?;
            EncryptionScheme::decrypt(&a.encrypted_post_state.ciphertext, &ss, &a.nullifier)
        })
        .map(|(_, acct)| acct)
        .filter(|acct| !acct.data.actor_state(program_account()).is_empty())
        .collect();
    let [acct] = opened.as_slice() else {
        panic!("one note of V opens, got {}", opened.len())
    };
    let ms: Multisig = borsh::from_slice(acct.data.actor_state(program_account())).unwrap();
    assert_eq!((ms.nonce, ms.threshold, ms.members.len()), (1, 2, 3));
    assert_eq!(acct.data.native_balance().unwrap(), 700);
}

// ---- image v2: Queue / ExecuteQueued (time lock), Spend (limit), worst-case quorum ----

const TL: u64 = 60_000;

fn time_locked() -> Fixture {
    funded_with(|ms| ms.time_lock = TL)
}

#[test]
fn time_lock_blocks_early_execute_then_executes_at_eta() {
    let mut f = time_locked();
    let now = f.chain.timestamp_ms;
    let pay = native_transfer(f.v, TO, 300);
    // an immediate Execute is refused in the guest under a time lock
    let e = f.execute(&[1, 2]).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::TimeLockMismatch), "{e}");
    let (t, expiry) = (now + 1_000, now + 10 * TL);
    let tx = f.run(signed_queue(f.v, &f.ms, pay.clone(), expiry, t, &[1, 2])).unwrap();
    let mut queued = f.ms.clone();
    let [.., dq, _] = msig_core::digests(&f.v, &f.ms, &pay, expiry, t);
    queued.queued = Some(msig_core::Queued { digest: dq, t, expiry });
    f.advance(&tx, queued.clone(), 1_000);

    let tx = f.run(signed_execute_queued(f.v, &f.ms, pay)).unwrap();
    f.chain.timestamp_ms = t + TL - 1;
    assert!(f.chain.settle(&tx).is_err(), "one ms before eta: outside the validity window");
    assert_eq!(f.chain.public_balance(TO), 0);
    f.chain.timestamp_ms = t + TL;
    let done = Multisig {
        nonce: 1,
        queued: None,
        ..queued
    };
    f.advance(&tx, done, 700);
    assert_eq!(f.chain.public_balance(TO), 300);
}

#[test]
fn queue_signature_cannot_execute_and_old_epoch_approvals_die() {
    let mut f = time_locked();
    let now = f.chain.timestamp_ms;
    let (t, expiry) = (now + 1_000, now + 10 * TL);
    // queue a removal of member 2; a payout is pre-approved for the next nonce at epoch 0
    let remove = Action::Config(ConfigOp::RemoveMember { key: member_key(2) });
    let tx = f
        .run(signed_queue(f.v, &f.ms, remove.clone(), expiry, t, &[1, 2]))
        .unwrap();
    let [.., dq, _] = msig_core::digests(&f.v, &f.ms, &remove, expiry, t);
    let mut queued = f.ms.clone();
    queued.queued = Some(msig_core::Queued { digest: dq, t, expiry });
    f.advance(&tx, queued.clone(), 1_000);
    let stale = signed_queue(
        f.v,
        &Multisig {
            nonce: 1,
            ..f.ms.clone()
        },
        native_transfer(f.v, TO, 300),
        expiry + TL,
        t + 1,
        &[0, 1],
    );
    // the published /queue signature replayed as the executor's
    let msig_core::Instruction::Queue { queuer, .. } =
        signed_queue(f.v, &f.ms, remove.clone(), expiry, t, &[1, 2])
    else {
        unreachable!()
    };
    let e = f
        .run(msig_core::Instruction::ExecuteQueued {
            action: remove.clone(),
            executor: queuer,
        })
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "/queue as /execute {e}");
    // the removal executes at eta: epoch 1, nonce 1, slot empty
    let tx = f.run(signed_execute_queued(f.v, &f.ms, remove)).unwrap();
    f.chain.timestamp_ms = t + TL;
    let mut ms1 = Multisig { queued: None, ..queued };
    ms1.members.remove(2);
    (ms1.config_epoch, ms1.nonce) = (1, 1);
    f.advance(&tx, ms1, 1_000);
    // approvals signed at epoch 0 for nonce 1 are dead
    let e = f.run(stale).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::BadSignature), "old epoch {e}");
}

const PERIOD: u64 = 86_400_000;

/// A vault with a 400-per-day limit, 1 SPEND signature; the chain clock is mid-way through day 10.
fn limited() -> Fixture {
    let mut f = funded_with(|ms| {
        ms.limit = Some(msig_core::Limit {
            amount: 400,
            period: PERIOD,
            threshold: 1,
        })
    });
    f.chain.timestamp_ms = 10 * PERIOD + PERIOD / 2;
    f
}

#[test]
fn spend_within_limit_over_limit_and_no_backfill() {
    let mut f = limited();
    let p = f.chain.timestamp_ms / PERIOD;
    let spend = |f: &Fixture, amount, p| f.run(signed_spend(f.v, &f.ms, TO, amount, p, &[1]));
    let tx = spend(&f, 250, p).unwrap();
    let mut ms = f.ms.clone();
    ms.spend = msig_core::SpendState {
        period_index: p,
        spent: 250,
        nonce: 1,
    };
    f.advance(&tx, ms, 750);
    assert_eq!(f.chain.public_balance(TO), 250);
    // over the period's limit (250 + 151 > 400)
    let e = spend(&f, 151, p).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::OverLimit), "{e}");
    // an earlier period cannot be claimed
    let e = spend(&f, 100, p - 1).unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::StalePeriod), "{e}");
    // the next period's limit only opens with that period: proven now, rejected by the window
    let tx = spend(&f, 400, p + 1).unwrap();
    assert!(f.chain.settle(&tx).is_err(), "next period not yet open");
    assert_eq!(f.chain.public_balance(TO), 250);
}

#[test]
fn spend_backfill_of_a_past_period_is_rejected_by_the_window() {
    // a vault that never spent (period_index 0) signs for an old period: the guest accepts the
    // arithmetic, the window [p·period, (p+1)·period) is in the past, so the tx cannot land
    let f = limited();
    let p = f.chain.timestamp_ms / PERIOD;
    let tx = f.run(signed_spend(f.v, &f.ms, TO, 400, p - 1, &[1])).unwrap();
    let mut chain = f.chain;
    assert!(chain.settle(&tx).is_err());
    assert_eq!(chain.public_balance(TO), 0);
}

#[test]
fn zero_threshold_and_zero_period_limits_are_rejected() {
    let f = funded();
    for (amount, period, threshold) in [(1, PERIOD, 0), (1, 0, 1), (1, PERIOD, 4)] {
        let set = Action::Config(ConfigOp::SetLimit(Some(msig_core::Limit {
            amount,
            period,
            threshold,
        })));
        let e = f
            .run(signed_execute(f.v, &f.ms, set, u64::MAX, &[1, 2]))
            .unwrap_err();
        assert_eq!(msig_error(&e), Some(Error::InvalidConfig), "{period} {threshold} {e}");
    }
    let e = f
        .run(signed_spend(f.v, &f.ms, TO, 1, 0, &[1]))
        .unwrap_err();
    assert_eq!(msig_error(&e), Some(Error::NoLimit), "{e}");
}

/// The most expensive instruction at the largest configurable quorum (N = M = MAX_MEMBERS,
/// a Config: M+2 verifications plus N key decodes in `validate`) stays within the public block
/// gas cap, so no configurable quorum is un-executable. Executor cycles (= LEZ gas), no proving.
#[test]
fn worst_case_quorum_fits_the_block_gas_cap() {
    use lee_core::program::ReceiveInput;
    use risc0_zkvm::{ExecutorEnv, default_executor};
    let n = u8::try_from(msig_core::MAX_MEMBERS).unwrap();
    let cfg = dev_cfg(n, n);
    let ms = initial_state(&cfg, &SALT);
    msig_core::validate(&ms).expect("N = M = MAX_MEMBERS is a valid config");
    let v = AccountId::new([0x55; 32]);
    let all: Vec<u8> = (0..n).collect();
    let cycles = |ix: msig_core::Instruction| {
        let input = ReceiveInput {
            receiver: Actor::new(v, program_account()),
            from: None,
            is_authorized: true,
            pre_state: borsh::to_vec(&ms).unwrap().into(),
            message: borsh::to_vec(&ix).unwrap().into(),
        };
        let mut env = ExecutorEnv::builder();
        env.write_slice(&lee_core::to_frame(&borsh::to_vec(&input).unwrap()));
        let info = default_executor().execute(env.build().unwrap(), MSIG_ELF).unwrap();
        assert!(matches!(info.exit_code, risc0_zkvm::ExitCode::Halted(0)), "{:?}", info.exit_code);
        info.cycles()
    };
    let config = cycles(signed_execute(
        v,
        &ms,
        Action::Config(ConfigOp::SetThreshold(n)),
        u64::MAX,
        &all,
    ));
    let call = cycles(signed_execute(v, &ms, native_transfer(v, TO, 1), u64::MAX, &all));
    println!("worst case N=M={n}: Config {config} cycles, Call {call} cycles, cap {MAX_GAS_EXEC}");
    assert!(config.max(call) <= MAX_GAS_EXEC, "Config {config} / Call {call} > {MAX_GAS_EXEC}");
}
