//! Native tests of the pure handler: Create and the §2.5 quorum rule of Execute.

use k256::schnorr::SigningKey;
use lee_core::account::{AccountId, Actor};
use lee_core::native_token::{Message as Native, NATIVE_TOKEN_PROGRAM_ID};
use lee_core::program::{PdaSeed, ReceiveInput};
use msig_core::*;

const P: AccountId = AccountId::new([0x50; 32]);
const TO: AccountId = AccountId::new([0x77; 32]);
const ALL: u8 = ALL_PERMS;

/// Dev key `i` (deterministic, test only).
fn sk(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i + 1; 32]).unwrap()
}

fn cfg(n: u8, m: u8) -> InitConfig {
    InitConfig {
        chain_tag: [7; 32],
        members: (0..n)
            .map(|i| Member {
                key: sk(i).verifying_key().to_bytes().into(),
                perms: ALL,
            })
            .collect(),
        threshold: m,
    }
}

fn v_of(c: &InitConfig) -> AccountId {
    AccountId::for_public_pda(&P, &PdaSeed::new(seed(c, &[1; 32])))
}

fn input(v: AccountId, pre: Vec<u8>, ix: &Instruction) -> ReceiveInput {
    ReceiveInput {
        receiver: Actor::new(v, P),
        from: None,
        is_authorized: false,
        pre_state: pre.into(),
        message: borsh::to_vec(ix).unwrap(),
    }
}

fn created(n: u8, m: u8) -> (AccountId, Multisig) {
    let c = cfg(n, m);
    let v = v_of(&c);
    let r = handle(&input(
        v,
        vec![],
        &Instruction::Create {
            cfg: c,
            salt: [1; 32],
            keys: None,
        },
    ))
    .unwrap();
    (v, borsh::from_slice(&r.post_state.unwrap()).unwrap())
}

fn transfer(v: AccountId, amount: u128) -> Action {
    Action::Call {
        to: Actor::new(v, NATIVE_TOKEN_PROGRAM_ID),
        data: borsh::to_vec(&Native::Transfer { to: TO, amount }).unwrap(),
        with_authority: true,
    }
}

fn sign(i: u8, d: &[u8; 32]) -> Sig {
    Sig {
        index: i,
        sig: sk(i).sign_raw(d, &[0; 32]).unwrap().to_bytes(),
    }
}

/// Execute signed by proposer 0, approvers `approvers`, executor 0 over `ms`'s current nonce.
fn execute(v: AccountId, ms: &Multisig, action: Action, approvers: &[u8]) -> Instruction {
    execute_by(v, ms, action, 0, approvers, 0)
}

/// Execute with an explicit proposer and executor (dev key = member index).
fn execute_by(
    v: AccountId,
    ms: &Multisig,
    action: Action,
    proposer: u8,
    approvers: &[u8],
    executor: u8,
) -> Instruction {
    let [dp, da, de] = execute_digests(&v, ms, &action, 1_000);
    Instruction::Execute {
        action,
        expiry: 1_000,
        proposer: sign(proposer, &dp),
        approvals: approvers.iter().map(|&i| sign(i, &da)).collect(),
        executor: sign(executor, &de),
    }
}

fn post(r: lee_core::program::Response) -> Multisig {
    borsh::from_slice(&r.post_state.unwrap()).unwrap()
}

fn run(
    v: AccountId,
    ms: &Multisig,
    ix: &Instruction,
) -> Result<lee_core::program::Response, Error> {
    handle(&input(v, borsh::to_vec(ms).unwrap(), ix))
}

#[test]
fn create_writes_config_at_nonce_zero() {
    let (_, ms) = created(3, 2);
    assert_eq!(
        (ms.threshold, ms.nonce, ms.config_epoch, ms.members.len()),
        (2, 0, 0, 3)
    );
}

#[test]
fn foreign_config_cannot_init() {
    // foreign_config_cannot_init: V commits to cfg, so another member set cannot initialise it
    let v = v_of(&cfg(3, 2));
    let ix = Instruction::Create {
        cfg: cfg(3, 1),
        salt: [1; 32],
        keys: None,
    };
    assert_eq!(
        handle(&input(v, vec![], &ix)).err(),
        Some(Error::WrongAccount)
    );
}

#[test]
fn create_on_initialised_account_fails() {
    // gms_holder_cannot_bypass_quorum (d)
    let c = cfg(3, 2);
    let (v, ms) = created(3, 2);
    let ix = Instruction::Create {
        cfg: c,
        salt: [1; 32],
        keys: None,
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::AlreadyInitialized));
}

#[test]
fn create_rejects_bad_threshold() {
    for m in [0, 4] {
        let c = cfg(3, m);
        let ix = Instruction::Create {
            cfg: c.clone(),
            salt: [1; 32],
            keys: None,
        };
        assert_eq!(
            handle(&input(v_of(&c), vec![], &ix)).err(),
            Some(Error::InvalidConfig)
        );
    }
}

#[test]
fn quorum_met_executes_and_bumps_nonce() {
    let (v, ms) = created(3, 2);
    let r = run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1, 2])).unwrap();
    let post: Multisig = borsh::from_slice(&r.post_state.unwrap()).unwrap();
    assert_eq!(post.nonce, 1);
    let [call] = r.sync_messages.as_slice() else {
        panic!("one call")
    };
    assert_eq!(call.to, Actor::new(v, NATIVE_TOKEN_PROGRAM_ID));
    assert!(call.pda_seeds.contains(&PdaSeed::new(ms.seed)));
    assert_eq!(r.validity.timestamps.end(), Some(1_000));
}

#[test]
fn below_threshold_rejected() {
    // below_threshold_rejected; gms_holder_cannot_bypass_quorum (b): M-1 approvals
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1])).err(),
        Some(Error::BelowThreshold)
    );
}

#[test]
fn duplicate_signer_rejected() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1, 1])).err(),
        Some(Error::UnsortedSigners)
    );
}

#[test]
fn unsorted_signers_are_rejected() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[2, 1])).err(),
        Some(Error::UnsortedSigners)
    );
}

#[test]
fn non_member_index_is_rejected() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1, 3])).err(),
        Some(Error::NotMember)
    );
}

#[test]
fn non_member_key_under_member_index_is_rejected() {
    let (v, ms) = created(3, 2);
    let action = transfer(v, 5);
    let Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = execute(v, &ms, action, &[1, 2])
    else {
        unreachable!()
    };
    let [_, da, _] = execute_digests(&v, &ms, &action, expiry);
    approvals[1].sig = sk(9).sign_raw(&da, &[0; 32]).unwrap().to_bytes();
    let ix = Instruction::Execute {
        action,
        expiry,
        proposer,
        approvals,
        executor,
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn proposer_without_vote_cannot_approve() {
    let (v, mut ms) = created(3, 2);
    ms.members[2].perms = INITIATE | EXECUTE;
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1, 2])).err(),
        Some(Error::MissingPermission)
    );
}

#[test]
fn replayed_approval_is_rejected() {
    let (v, ms) = created(3, 2);
    let ix = execute(v, &ms, transfer(v, 5), &[1, 2]);
    let r = run(v, &ms, &ix).unwrap();
    let after: Multisig = borsh::from_slice(&r.post_state.unwrap()).unwrap();
    assert_eq!(run(v, &after, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn tampered_signature_is_rejected() {
    let (v, ms) = created(3, 2);
    let Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = execute(v, &ms, transfer(v, 5), &[1, 2])
    else {
        unreachable!()
    };
    approvals[0].sig[10] ^= 1;
    let ix = Instruction::Execute {
        action,
        expiry,
        proposer,
        approvals,
        executor,
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn wrong_action_rejected() {
    // wrong_action_rejected: signatures bind sha256(borsh(action))
    let (v, ms) = created(3, 2);
    let Instruction::Execute {
        expiry,
        proposer,
        approvals,
        executor,
        ..
    } = execute(v, &ms, transfer(v, 5), &[1, 2])
    else {
        unreachable!()
    };
    let ix = Instruction::Execute {
        action: transfer(v, 500),
        expiry,
        proposer,
        approvals,
        executor,
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn uninitialised_vault_cannot_be_drained() {
    let (v, ms) = created(3, 2);
    for action in [
        transfer(v, 5),
        Action::Config(ConfigOp::SetThreshold(1)),
        Action::Cancel,
    ] {
        let ix = execute(v, &ms, action, &[1, 2]);
        assert_eq!(
            handle(&input(v, vec![], &ix)).err(),
            Some(Error::Uninitialized)
        );
    }
}

#[test]
fn authority_only_for_v_and_no_reentry() {
    let (v, ms) = created(3, 2);
    let foreign = Action::Call {
        to: Actor::new(TO, NATIVE_TOKEN_PROGRAM_ID),
        data: vec![],
        with_authority: true,
    };
    assert_eq!(
        run(v, &ms, &execute(v, &ms, foreign, &[1, 2])).err(),
        Some(Error::AuthorityOnForeignAccount)
    );
    let reenter = Action::Call {
        to: Actor::new(v, P),
        data: vec![],
        with_authority: false,
    };
    assert_eq!(
        run(v, &ms, &execute(v, &ms, reenter, &[1, 2])).err(),
        Some(Error::Reentry)
    );
}

#[test]
fn replies_keep_state() {
    let (v, ms) = created(3, 2);
    let mut i = input(
        v,
        borsh::to_vec(&ms).unwrap(),
        &execute(v, &ms, transfer(v, 5), &[1]),
    );
    i.from = Some(Actor::new(v, NATIVE_TOKEN_PROGRAM_ID));
    assert_eq!(handle(&i).unwrap().post_state, None);
}

// ---- §2.3 Config / Cancel, §2.5 roles, §2.6 R.2 cutoff ----

fn remove(i: u8) -> Action {
    Action::Config(ConfigOp::RemoveMember {
        key: sk(i).verifying_key().to_bytes().into(),
    })
}

#[test]
fn config_change_needs_quorum() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, remove(1), &[1])).err(),
        Some(Error::BelowThreshold)
    );
}

#[test]
fn remove_member_bumps_epoch_and_nonce() {
    let (v, ms) = created(3, 2);
    let r = run(v, &ms, &execute(v, &ms, remove(1), &[1, 2])).unwrap();
    assert!(r.sync_messages.is_empty());
    assert_eq!(r.validity.timestamps.end(), Some(1_000));
    let after = post(r);
    assert_eq!((after.config_epoch, after.nonce), (1, 1));
    let keys: Vec<_> = after.members.iter().map(|m| m.key).collect();
    let k = |i: u8| -> [u8; 32] { sk(i).verifying_key().to_bytes().into() };
    assert_eq!(keys, vec![k(0), k(2)], "member 1 removed, order kept");
}

#[test]
fn add_member_and_set_threshold() {
    let (v, ms) = created(3, 2);
    let add = Action::Config(ConfigOp::AddMember(Member {
        key: sk(3).verifying_key().to_bytes().into(),
        perms: VOTE,
    }));
    let ms = post(run(v, &ms, &execute(v, &ms, add, &[1, 2])).unwrap());
    assert_eq!((ms.members.len(), ms.config_epoch, ms.nonce), (4, 1, 1));
    let set = Action::Config(ConfigOp::SetThreshold(4));
    let ms = post(run(v, &ms, &execute(v, &ms, set, &[0, 3])).unwrap());
    assert_eq!((ms.threshold, ms.config_epoch, ms.nonce), (4, 2, 2));
    // the new VOTE-only member counts toward the new threshold
    let ms2 = post(run(v, &ms, &execute(v, &ms, Action::Cancel, &[0, 1, 2, 3])).unwrap());
    assert_eq!(ms2.nonce, 3);
}

#[test]
fn config_breaking_invariants_is_rejected() {
    let (v, ms) = created(3, 2);
    let dup = Action::Config(ConfigOp::AddMember(ms.members[0].clone()));
    let bad_key = Action::Config(ConfigOp::AddMember(Member {
        key: [0xff; 32],
        perms: VOTE,
    }));
    for action in [
        Action::Config(ConfigOp::SetThreshold(0)),
        Action::Config(ConfigOp::SetThreshold(4)),
        dup,
        bad_key,
    ] {
        assert_eq!(
            run(v, &ms, &execute(v, &ms, action, &[1, 2])).err(),
            Some(Error::InvalidConfig)
        );
    }
    // 2-of-2 after one removal is fine; a second removal leaves 1 voter < M
    let ms = post(run(v, &ms, &execute(v, &ms, remove(2), &[1, 2])).unwrap());
    assert_eq!(
        run(v, &ms, &execute(v, &ms, remove(1), &[0, 1])).err(),
        Some(Error::InvalidConfig)
    );
    let not_member = Action::Config(ConfigOp::RemoveMember { key: [9; 32] });
    assert_eq!(
        run(v, &ms, &execute(v, &ms, not_member, &[0, 1])).err(),
        Some(Error::NotMember)
    );
}

#[test]
fn old_epoch_approvals_rejected() {
    // R.2: approvals signed at epoch 0 die when a Config lands, including approvals pre-signed for
    // the next nonce and any approval by the removed member
    let (v, ms0) = created(3, 2);
    let mut next = ms0.clone();
    next.nonce = 1; // pre-signed for the nonce after the removal, still at epoch 0
    let presigned = execute(v, &next, transfer(v, 5), &[1, 2]);
    let ms1 = post(run(v, &ms0, &execute(v, &ms0, remove(1), &[1, 2])).unwrap());
    assert_eq!((ms1.config_epoch, ms1.nonce), (1, 1));
    assert_eq!(run(v, &ms1, &presigned).err(), Some(Error::BadSignature));
    // the removed member signs afresh at epoch 1 under its old index (now member 2's key)
    assert_eq!(
        run(v, &ms1, &execute(v, &ms1, transfer(v, 5), &[0, 1])).err(),
        Some(Error::BadSignature)
    );
    // and under its old index 2 shifted past the end
    let Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = execute(v, &ms1, transfer(v, 5), &[0, 1])
    else {
        unreachable!()
    };
    let [_, da, _] = execute_digests(&v, &ms1, &action, expiry);
    approvals[1] = Sig {
        index: 2,
        sig: sk(1).sign_raw(&da, &[0; 32]).unwrap().to_bytes(),
    };
    let ix = Instruction::Execute {
        action,
        expiry,
        proposer,
        approvals,
        executor,
    };
    assert_eq!(run(v, &ms1, &ix).err(), Some(Error::NotMember));
    // the remaining members (old member 2 is now index 1) re-approve at epoch 1
    let mut fresh = execute_by(v, &ms1, transfer(v, 5), 0, &[], 0);
    if let Instruction::Execute {
        action, approvals, ..
    } = &mut fresh
    {
        let [_, da, _] = execute_digests(&v, &ms1, action, 1_000);
        *approvals = vec![
            sign(0, &da),
            Sig {
                index: 1,
                sig: sk(2).sign_raw(&da, &[0; 32]).unwrap().to_bytes(),
            },
        ];
    }
    assert_eq!(post(run(v, &ms1, &fresh).unwrap()).nonce, 2);
}

#[test]
fn cancel_consumes_nonce_and_kills_competing_proposals() {
    let (v, ms) = created(3, 2);
    let pending = execute(v, &ms, transfer(v, 5), &[1, 2]);
    let r = run(v, &ms, &execute(v, &ms, Action::Cancel, &[1, 2])).unwrap();
    assert!(r.sync_messages.is_empty());
    let after = post(r);
    assert_eq!((after.nonce, after.config_epoch), (1, 0));
    assert_eq!(
        Multisig { nonce: 0, ..after.clone() },
        ms,
        "Cancel changes only the nonce"
    );
    assert_eq!(run(v, &after, &pending).err(), Some(Error::BadSignature));
}

#[test]
fn cancel_needs_quorum() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute(v, &ms, Action::Cancel, &[2])).err(),
        Some(Error::BelowThreshold)
    );
}

#[test]
fn proposer_without_initiate_rejected() {
    let (v, mut ms) = created(3, 2);
    ms.members[1].perms = VOTE | EXECUTE;
    assert_eq!(
        run(v, &ms, &execute_by(v, &ms, Action::Cancel, 1, &[1, 2], 0)).err(),
        Some(Error::MissingPermission)
    );
}

#[test]
fn non_executor_cannot_execute() {
    let (v, mut ms) = created(3, 2);
    ms.members[2].perms = INITIATE | VOTE;
    assert_eq!(
        run(v, &ms, &execute_by(v, &ms, Action::Cancel, 0, &[1, 2], 2)).err(),
        Some(Error::MissingPermission)
    );
    // the same member as proposer is fine
    assert!(run(v, &ms, &execute_by(v, &ms, Action::Cancel, 2, &[1, 2], 0)).is_ok());
}

#[test]
fn approvals_are_attributed_to_the_member_at_their_index() {
    // R.1 attribution: member 2's signature presented as member 1's does not count
    let (v, ms) = created(3, 2);
    let Instruction::Execute {
        action,
        expiry,
        proposer,
        mut approvals,
        executor,
    } = execute(v, &ms, Action::Cancel, &[1, 2])
    else {
        unreachable!()
    };
    approvals[0].sig = approvals[1].sig;
    let ix = Instruction::Execute {
        action,
        expiry,
        proposer,
        approvals,
        executor,
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn proposer_does_not_count_as_approver() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &execute_by(v, &ms, Action::Cancel, 0, &[1], 0)).err(),
        Some(Error::BelowThreshold)
    );
}

#[test]
fn call_action_encoding_is_unchanged() {
    // Action::Call stays borsh variant 0, so Execute signatures from before Config/Cancel hold
    let a = transfer(TO, 1);
    assert_eq!(borsh::to_vec(&a).unwrap()[0], 0);
    assert_eq!(borsh::to_vec(&Action::Cancel).unwrap(), vec![2]);
}

/// Every approver subset x every valid threshold against a trivial oracle. Member `NOVOTE`
/// lacks VOTE, so the oracle is "no non-voter in the subset and |subset| >= M"; the filtered run
/// (non-voters dropped) must pass iff |subset ∩ VOTE| >= M.
fn exhaustive_subsets(n: u8) {
    const NOVOTE: u8 = 3;
    for m in 1..n {
        let mut c = cfg(n, m);
        c.members[usize::from(NOVOTE)].perms = INITIATE | EXECUTE;
        let v = v_of(&c);
        let ms = Multisig::new(&c, seed(&c, &[1; 32]));
        for mask in 0u32..(1 << n) {
            let all: Vec<u8> = (0..n).filter(|i| mask >> i & 1 == 1).collect();
            let voters: Vec<u8> = all.iter().copied().filter(|&i| i != NOVOTE).collect();
            let r = run(v, &ms, &execute(v, &ms, Action::Cancel, &all));
            let ok = !all.contains(&NOVOTE) && all.len() >= usize::from(m);
            assert_eq!(r.is_ok(), ok, "n={n} m={m} subset={all:?}: {:?}", r.err());
            let r = run(v, &ms, &execute(v, &ms, Action::Cancel, &voters));
            let ok = voters.len() >= usize::from(m);
            assert_eq!(r.is_ok(), ok, "n={n} m={m} voters={voters:?}: {:?}", r.err());
        }
    }
}

#[test]
fn exhaustive_subset_oracle_n5() {
    exhaustive_subsets(5);
}

#[test]
fn exhaustive_subset_oracle_n7() {
    exhaustive_subsets(7);
}

/// Frozen known answers of the three Execute digests (fixed V, epoch 3, nonce 9, expiry 1000)
/// for each Action variant. Expected values computed independently with Python hashlib from
/// ARCHITECTURE §2.4; any change to a tag, field order, encoding or padding fails here.
#[test]
fn approval_digests_known_answers() {
    let v = AccountId::new([0x42; 32]);
    let ms = Multisig {
        config_epoch: 3,
        nonce: 9,
        ..Multisig::new(
            &InitConfig {
                chain_tag: [0; 32],
                members: vec![],
                threshold: 1,
            },
            [0; 32],
        )
    };
    let call = Action::Call {
        to: Actor::new(AccountId::new([1; 32]), AccountId::new([2; 32])),
        data: vec![1, 2, 3],
        with_authority: true,
    };
    let cases: [(&str, Action, [&str; 3]); 3] = [
        ("call", call, [
            "64aa9148b595abe7db60e75bb7d1befae0b417765a81ff7829bdd86ac9134e66",
            "f16f192123ce824da39d3a4659ec14296f24c9ff350855ec8f704b751d81e9b3",
            "3245a7d1cff523d3f8a8f18f22158e0ffbfec3aafac06adfd6132aa434786fc3",
        ]),
        ("config", Action::Config(ConfigOp::SetThreshold(2)), [
            "d809b3987f5917cc9a933b9f2ba76ae1bd990f6e15378e08a53645faba3f65cc",
            "f3779d5e24a44663331e82cc380b053bb4ebfabef8088493b25b11d460015fe8",
            "c4a4c48cd30d3a11daf1eaa10d0a29644cbd94d4c2a7a2b802f08ad789ff2895",
        ]),
        ("cancel", Action::Cancel, [
            "7715ce37a62887d5d62523ae1fc09b04b238fee3504ec6828f81504e6e27c285",
            "e6e6cd32fddbea30e62aecba0c2c7fbf24d1f5367a4db3374c7885fda04cdc82",
            "8b838b1016b5dd4c5f5fb722cd9c0b5c03c266ac6472aa309cb915ddcd654e0b",
        ]),
    ];
    for (name, action, want) in cases {
        let got = execute_digests(&v, &ms, &action, 1000);
        for (g, w) in got.iter().zip(want) {
            let hex: String = g.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, w, "{name}");
        }
    }
}

fn hex32(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Image v2: frozen `/queue` and `/execute` digests over `body ‖ t` (t = 500; same V, epoch
/// 3, nonce 9, expiry 1000 as above) and the `/spend` digest (spend nonce 4, p 7, to 0x77.., amount
/// 150). Expected values computed independently with Python hashlib from msig-core's documented
/// layouts (the same script reproduces the Execute answers above).
#[test]
fn queue_and_spend_digests_known_answers() {
    let v = AccountId::new([0x42; 32]);
    let mut ms = Multisig {
        config_epoch: 3,
        nonce: 9,
        ..Multisig::new(
            &InitConfig {
                chain_tag: [0; 32],
                members: vec![],
                threshold: 1,
            },
            [0; 32],
        )
    };
    let limit = Limit {
        amount: 150,
        period: 86_400_000,
        threshold: 1,
    };
    let cases: [(&str, Action, [&str; 2]); 3] = [
        ("cancel", Action::Cancel, [
            "e9736fd028e3740d66d64fc1ddb88289c93ce4d377af2abd017881bffbdd6312",
            "feb6c4e72e07dac9a9599c2070430e3d8fd6af909f5ada9e9ad399c7076abd4d",
        ]),
        ("set_time_lock", Action::Config(ConfigOp::SetTimeLock(5000)), [
            "92e1f4434f49c592b7120e2ad84f341b951f50313de0fcf770748a5aaefc0692",
            "016a40392dbb706415f12fbaa4b57b0bb0b5a5eda8c1f38a4db6991be0a15738",
        ]),
        ("set_limit", Action::Config(ConfigOp::SetLimit(Some(limit))), [
            "635a4db86adb381ceef5a301f13976e7ecd77d7e482284f5b214e0a30c45b0d5",
            "a88ca725c8a9fa2fa1ceb95b3c300ab4b5865c2eb5550dc8803f9391e2fca1cb",
        ]),
    ];
    for (name, action, [wq, we]) in cases {
        let [_, _, dq, de] = digests(&v, &ms, &action, 1000, 500);
        assert_eq!((hex32(&dq), hex32(&de)), (wq.to_string(), we.to_string()), "{name}");
    }
    ms.spend.nonce = 4;
    assert_eq!(
        hex32(&spend_digest(&v, &ms, &TO, 150, 7)),
        "cb8327327aba56605b6f6fafa2503e5c9bbce5102390e30d628a3a497bf71af6"
    );
}

// ---- perms, MAX_MEMBERS, state version ----

fn create_err(c: InitConfig) -> Option<Error> {
    let ix = Instruction::Create {
        cfg: c.clone(),
        salt: [1; 32],
        keys: None,
    };
    handle(&input(v_of(&c), vec![], &ix)).err()
}

#[test]
fn zero_and_unknown_perm_bits_are_rejected() {
    for perms in [0, 16, 0x80 | ALL] {
        let mut c = cfg(3, 2);
        c.members[2].perms = perms;
        assert_eq!(create_err(c), Some(Error::InvalidConfig), "create perms={perms:#x}");
    }
    let (v, ms) = created(3, 2);
    for perms in [0, 0x40] {
        let add = Action::Config(ConfigOp::AddMember(Member {
            key: sk(3).verifying_key().to_bytes().into(),
            perms,
        }));
        assert_eq!(
            run(v, &ms, &execute(v, &ms, add, &[1, 2])).err(),
            Some(Error::InvalidConfig),
            "add perms={perms:#x}"
        );
    }
}

#[test]
fn member_count_is_capped_at_max_members() {
    let n = u8::try_from(MAX_MEMBERS).unwrap();
    assert!(create_err(cfg(n, n)).is_none(), "N = MAX_MEMBERS, M = N is configurable");
    assert_eq!(create_err(cfg(n + 1, 2)), Some(Error::InvalidConfig));
}

/// The v1 layout (image v1): no time lock, limit, queue or spend fields.
#[derive(borsh::BorshSerialize)]
struct MultisigV1 {
    version: u8,
    chain_tag: [u8; 32],
    seed: [u8; 32],
    members: Vec<Member>,
    threshold: u8,
    config_epoch: u64,
    nonce: u64,
}

#[test]
fn v1_state_is_rejected_not_migrated() {
    let (v, ms) = created(3, 2);
    let old = MultisigV1 {
        version: 1,
        chain_tag: ms.chain_tag,
        seed: ms.seed,
        members: ms.members.clone(),
        threshold: ms.threshold,
        config_epoch: 0,
        nonce: 0,
    };
    let ix = execute(v, &ms, Action::Cancel, &[1, 2]);
    let r = handle(&input(v, borsh::to_vec(&old).unwrap(), &ix));
    assert_eq!(r.err(), Some(Error::UnsupportedVersion));
    // Create on a v1 account is still AlreadyInitialized, never an overwrite
    let c = cfg(3, 2);
    let create = Instruction::Create {
        cfg: c,
        salt: [1; 32],
        keys: None,
    };
    let r = handle(&input(v, borsh::to_vec(&old).unwrap(), &create));
    assert_eq!(r.err(), Some(Error::AlreadyInitialized));
    assert_eq!(ms.version, 2);
}

// ---- §2.3 Queue / ExecuteQueued (time lock, one slot) ----

const TL: u64 = 100;

/// A 3-of-2 vault with `time_lock = TL`, set through an immediate Config (epoch 1, nonce 1).
fn timelocked() -> (AccountId, Multisig) {
    let (v, ms) = created(3, 2);
    let set = Action::Config(ConfigOp::SetTimeLock(TL));
    let ms = post(run(v, &ms, &execute(v, &ms, set, &[1, 2])).unwrap());
    assert_eq!((ms.time_lock, ms.config_epoch, ms.nonce), (TL, 1, 1));
    (v, ms)
}

fn queue_by(
    v: AccountId,
    ms: &Multisig,
    action: Action,
    expiry: u64,
    t: u64,
    approvers: &[u8],
    queuer: u8,
) -> Instruction {
    let [dp, da, dq, _] = digests(&v, ms, &action, expiry, t);
    Instruction::Queue {
        action,
        expiry,
        t,
        proposer: sign(0, &dp),
        approvals: approvers.iter().map(|&i| sign(i, &da)).collect(),
        queuer: sign(queuer, &dq),
    }
}

fn queue(v: AccountId, ms: &Multisig, action: Action, t: u64) -> Instruction {
    queue_by(v, ms, action, 1_000, t, &[1, 2], 0)
}

/// `ExecuteQueued` of `action` signed by executor 0 over `ms.queued`'s `t` and expiry.
fn execute_queued(v: AccountId, ms: &Multisig, action: Action) -> Instruction {
    let q = ms.queued.clone().expect("queued");
    let [.., de] = digests(&v, ms, &action, q.expiry, q.t);
    Instruction::ExecuteQueued {
        action,
        executor: sign(0, &de),
    }
}

#[test]
fn time_lock_blocks_immediate_execute_but_not_cancel() {
    let (v, ms) = timelocked();
    assert_eq!(
        run(v, &ms, &execute(v, &ms, transfer(v, 5), &[1, 2])).err(),
        Some(Error::TimeLockMismatch)
    );
    assert_eq!(
        run(v, &ms, &execute(v, &ms, remove(1), &[1, 2])).err(),
        Some(Error::TimeLockMismatch),
        "config changes wait the delay too"
    );
    assert_eq!(post(run(v, &ms, &execute(v, &ms, Action::Cancel, &[1, 2])).unwrap()).nonce, 2);
}

#[test]
fn queue_without_time_lock_is_rejected() {
    let (v, ms) = created(3, 2);
    assert_eq!(
        run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).err(),
        Some(Error::TimeLockMismatch)
    );
}

#[test]
fn queue_then_execute_after_eta() {
    let (v, ms) = timelocked();
    let r = run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).unwrap();
    assert!(r.sync_messages.is_empty(), "Queue dispatches nothing");
    assert_eq!(
        (r.validity.timestamps.start(), r.validity.timestamps.end()),
        (None, Some(500)),
        "Queue must land before t"
    );
    let q = post(r);
    assert_eq!(q.nonce, ms.nonce, "Queue does not consume the nonce");
    let entry = q.queued.clone().unwrap();
    assert_eq!((entry.t, entry.expiry), (500, 1_000));
    let r = run(v, &q, &execute_queued(v, &q, transfer(v, 5))).unwrap();
    assert_eq!(
        (r.validity.timestamps.start(), r.validity.timestamps.end()),
        (Some(500 + TL), Some(1_000)),
        "time lock: valid only from eta = t + time_lock"
    );
    assert_eq!(r.sync_messages.len(), 1);
    let after = post(r);
    assert_eq!((after.nonce, after.queued.clone()), (ms.nonce + 1, None));
    // replay of the same ExecuteQueued: slot is empty
    assert_eq!(
        run(v, &after, &execute_queued(v, &q, transfer(v, 5))).err(),
        Some(Error::NotQueued)
    );
}

#[test]
fn queue_window_checks() {
    let (v, ms) = timelocked();
    // eta = t + time_lock must be < expiry
    assert_eq!(
        run(v, &ms, &queue(v, &ms, transfer(v, 5), 1_000 - TL)).err(),
        Some(Error::BadWindow)
    );
    // checked add
    assert_eq!(
        run(v, &ms, &queue_by(v, &ms, transfer(v, 5), u64::MAX, u64::MAX - 1, &[1, 2], 0)).err(),
        Some(Error::Overflow)
    );
    // the single slot: a live entry blocks a second Queue; after its expiry it may be overwritten
    let q = post(run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).unwrap());
    assert_eq!(
        run(v, &q, &queue_by(v, &q, transfer(v, 6), 2_000, 900, &[1, 2], 0)).err(),
        Some(Error::QueueOccupied)
    );
    let r = run(v, &q, &queue_by(v, &q, transfer(v, 6), 2_000, 1_500, &[1, 2], 0)).unwrap();
    assert_eq!(
        (r.validity.timestamps.start(), r.validity.timestamps.end()),
        (Some(1_000), Some(1_500)),
        "overwrite only once the old entry has expired"
    );
}

#[test]
fn queuer_must_be_initiate_and_execute_must_match_the_queued_action() {
    let (v, ms) = timelocked();
    // the queuer is the proposer, so it needs INITIATE as proposer and as queuer
    let mut no_init = ms.clone();
    no_init.members[0].perms = VOTE | EXECUTE;
    assert_eq!(
        run(v, &no_init, &queue(v, &no_init, transfer(v, 5), 500)).err(),
        Some(Error::MissingPermission)
    );
    let q = post(run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).unwrap());
    let other = execute_queued(v, &q, transfer(v, 500));
    assert_eq!(run(v, &q, &other).err(), Some(Error::NotQueued));
}

#[test]
fn queue_and_execute_signatures_are_not_interchangeable() {
    let (v, ms) = timelocked();
    let action = transfer(v, 5);
    let [dp, da, dq, de] = digests(&v, &ms, &action, 1_000, 500);
    let approvals: Vec<Sig> = [1, 2].iter().map(|&i| sign(i, &da)).collect();
    // an /execute signature presented as the queuer's
    let ix = Instruction::Queue {
        action: action.clone(),
        expiry: 1_000,
        t: 500,
        proposer: sign(0, &dp),
        approvals,
        queuer: sign(0, &de),
    };
    assert_eq!(run(v, &ms, &ix).err(), Some(Error::BadSignature));
    // the published /queue signature presented as the executor's
    let q = post(run(v, &ms, &queue(v, &ms, action.clone(), 500)).unwrap());
    let ix = Instruction::ExecuteQueued {
        action: action.clone(),
        executor: sign(0, &dq),
    };
    assert_eq!(run(v, &q, &ix).err(), Some(Error::BadSignature));
    // an immediate-Execute signature (t = 0) does not execute an entry queued at t = 500
    let [.., de0] = digests(&v, &q, &action, 1_000, 0);
    let ix = Instruction::ExecuteQueued {
        action,
        executor: sign(0, &de0),
    };
    assert_eq!(run(v, &q, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn queued_action_is_voided_by_cancel_and_by_an_epoch_bump() {
    let (v, ms) = timelocked();
    let q = post(run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).unwrap());
    // Cancel (immediate under the time lock) clears the slot
    let c = post(run(v, &q, &execute(v, &q, Action::Cancel, &[1, 2])).unwrap());
    assert_eq!(c.queued, None);
    assert_eq!(
        run(v, &c, &execute_queued(v, &q, transfer(v, 5))).err(),
        Some(Error::NotQueued)
    );
    // the queued digest binds config_epoch: a member removal (epoch + 1) under the same entry,
    // nonce unchanged, voids it (defense in depth: with one slot no Config lands while it is live)
    let mut removed = q.clone();
    removed.members.remove(1);
    removed.config_epoch += 1;
    assert_eq!(
        run(v, &removed, &execute_queued(v, &q, transfer(v, 5))).err(),
        Some(Error::NotQueued)
    );
}

#[test]
fn queued_member_removal_executes_and_kills_old_epoch_approvals() {
    let (v, ms) = timelocked();
    let q = post(run(v, &ms, &queue(v, &ms, remove(2), 500)).unwrap());
    let mut next = q.clone();
    next.nonce += 1; // approvals pre-signed for the next nonce at the old epoch
    let presigned = |approvers: &[u8]| queue_by(v, &next, transfer(v, 5), 1_000, 600, approvers, 0);
    let after = post(run(v, &q, &execute_queued(v, &q, remove(2))).unwrap());
    assert_eq!(
        (after.members.len(), after.config_epoch, after.nonce, after.queued.clone()),
        (2, 2, 2, None)
    );
    assert_eq!(run(v, &after, &presigned(&[1, 2])).err(), Some(Error::BadSignature));
    assert_eq!(run(v, &after, &presigned(&[0, 1])).err(), Some(Error::BadSignature));
    assert_eq!(
        run(v, &after, &queue(v, &after, transfer(v, 5), 600)).err(),
        Some(Error::NotMember),
        "removed member 2 approves afresh"
    );
    let fresh = queue_by(v, &after, transfer(v, 5), 1_000, 600, &[0, 1], 0);
    assert!(run(v, &after, &fresh).is_ok(), "remaining members re-approve at the new epoch");
}

// ---- §2.3 Spend under the limit ----

const PERIOD: u64 = 1_000;

fn limit(amount: u128, threshold: u8) -> Action {
    Action::Config(ConfigOp::SetLimit(Some(Limit {
        amount,
        period: PERIOD,
        threshold,
    })))
}

/// A 3-of-2 vault with a 100-per-period limit, 1 SPEND signature.
fn limited() -> (AccountId, Multisig) {
    let (v, ms) = created(3, 2);
    let ms = post(run(v, &ms, &execute(v, &ms, limit(100, 1), &[1, 2])).unwrap());
    (v, ms)
}

fn spend(v: AccountId, ms: &Multisig, amount: u128, p: u64, signers: &[u8]) -> Instruction {
    let d = spend_digest(&v, ms, &TO, amount, p);
    Instruction::Spend {
        to: TO,
        amount,
        p,
        signers: signers.iter().map(|&i| sign(i, &d)).collect(),
    }
}

#[test]
fn spend_within_limit_moves_native_and_keeps_the_action_nonce() {
    let (v, ms) = limited();
    let r = run(v, &ms, &spend(v, &ms, 60, 7, &[1])).unwrap();
    assert_eq!(
        (r.validity.timestamps.start(), r.validity.timestamps.end()),
        (Some(7 * PERIOD), Some(8 * PERIOD))
    );
    let [call] = r.sync_messages.as_slice() else {
        panic!("one call")
    };
    assert_eq!(call.to, Actor::new(v, NATIVE_TOKEN_PROGRAM_ID));
    assert!(call.pda_seeds.contains(&PdaSeed::new(ms.seed)));
    let after = post(r);
    assert_eq!(after.spend, SpendState { period_index: 7, spent: 60, nonce: 1 });
    assert_eq!((after.nonce, after.config_epoch), (ms.nonce, ms.config_epoch));
    // the rest of the period's limit
    let after = post(run(v, &after, &spend(v, &after, 40, 7, &[2])).unwrap());
    assert_eq!(after.spend.spent, 100);
    // replay of a spend: its nonce moved on
    let ix = spend(v, &ms, 60, 7, &[1]);
    assert_eq!(run(v, &after, &ix).err(), Some(Error::BadSignature));
}

#[test]
fn spend_over_limit_is_rejected_and_resets_on_a_new_period() {
    let (v, ms) = limited();
    assert_eq!(run(v, &ms, &spend(v, &ms, 101, 7, &[1])).err(), Some(Error::OverLimit));
    let a = post(run(v, &ms, &spend(v, &ms, 70, 7, &[1])).unwrap());
    assert_eq!(run(v, &a, &spend(v, &a, 31, 7, &[1])).err(), Some(Error::OverLimit));
    assert_eq!(
        run(v, &a, &spend(v, &a, u128::MAX, 7, &[1])).err(),
        Some(Error::Overflow)
    );
    let b = post(run(v, &a, &spend(v, &a, 100, 8, &[1])).unwrap());
    assert_eq!(b.spend, SpendState { period_index: 8, spent: 100, nonce: 2 });
}

#[test]
fn spend_period_cannot_be_backfilled() {
    let (v, ms) = limited();
    let a = post(run(v, &ms, &spend(v, &ms, 100, 8, &[1])).unwrap());
    assert_eq!(run(v, &a, &spend(v, &a, 50, 7, &[1])).err(), Some(Error::StalePeriod));
    // p·period overflows: no window
    assert_eq!(
        run(v, &a, &spend(v, &a, 1, u64::MAX, &[1])).err(),
        Some(Error::Overflow)
    );
}

#[test]
fn spend_needs_a_limit_and_enough_spend_members() {
    let (v, ms) = created(3, 2);
    assert_eq!(run(v, &ms, &spend(v, &ms, 1, 0, &[1])).err(), Some(Error::NoLimit));
    let ms = post(run(v, &ms, &execute(v, &ms, limit(100, 2), &[1, 2])).unwrap());
    assert_eq!(run(v, &ms, &spend(v, &ms, 1, 0, &[1])).err(), Some(Error::BelowThreshold));
    assert_eq!(run(v, &ms, &spend(v, &ms, 1, 0, &[2, 1])).err(), Some(Error::UnsortedSigners));
    assert!(run(v, &ms, &spend(v, &ms, 1, 0, &[1, 2])).is_ok());
    let mut no_spend = ms.clone();
    no_spend.members[2].perms = INITIATE | VOTE | EXECUTE;
    assert_eq!(
        run(v, &no_spend, &spend(v, &no_spend, 1, 0, &[1, 2])).err(),
        Some(Error::MissingPermission)
    );
}

#[test]
fn bad_limits_are_rejected() {
    let (v, ms) = created(3, 2);
    let zero_period = Action::Config(ConfigOp::SetLimit(Some(Limit {
        amount: 1,
        period: 0,
        threshold: 1,
    })));
    for bad in [limit(1, 0), limit(1, 4), zero_period] {
        assert_eq!(
            run(v, &ms, &execute(v, &ms, bad, &[1, 2])).err(),
            Some(Error::InvalidConfig)
        );
    }
    // removing the last SPEND members below limit.threshold is rejected too
    let ms = post(run(v, &ms, &execute(v, &ms, limit(1, 3), &[1, 2])).unwrap());
    assert_eq!(
        run(v, &ms, &execute(v, &ms, remove(2), &[1, 2])).err(),
        Some(Error::InvalidConfig)
    );
    // and the limit can be dropped
    let ms = post(run(v, &ms, &execute(v, &ms, Action::Config(ConfigOp::SetLimit(None)), &[1, 2])).unwrap());
    assert_eq!(ms.limit, None);
}

// ---- image v2.1 fixes: time lock cap, SetLimit period reset, proposer-only queue ----

/// A time lock above MAX_TIME_LOCK would make every later Queue impossible (t + time_lock never
/// < expiry) while SetTimeLock(0) itself has to be queued: rejected. At the cap the vault still works.
#[test]
fn time_lock_is_capped_and_a_max_lock_still_queues_and_executes() {
    let (v, ms) = created(3, 2);
    for bad in [MAX_TIME_LOCK + 1, u64::MAX] {
        let set = Action::Config(ConfigOp::SetTimeLock(bad));
        assert_eq!(run(v, &ms, &execute(v, &ms, set, &[1, 2])).err(), Some(Error::InvalidConfig));
    }
    let set = Action::Config(ConfigOp::SetTimeLock(MAX_TIME_LOCK));
    let ms = post(run(v, &ms, &execute(v, &ms, set, &[1, 2])).unwrap());
    assert_eq!(ms.time_lock, MAX_TIME_LOCK);
    let (t, expiry) = (1_000, 1_000 + MAX_TIME_LOCK + 1);
    let unlock = Action::Config(ConfigOp::SetTimeLock(0));
    let q = post(run(v, &ms, &queue_by(v, &ms, unlock.clone(), expiry, t, &[1, 2], 0)).unwrap());
    let r = run(v, &q, &execute_queued(v, &q, unlock)).unwrap();
    assert_eq!(
        (r.validity.timestamps.start(), r.validity.timestamps.end()),
        (Some(t + MAX_TIME_LOCK), Some(expiry))
    );
    assert_eq!(post(r).time_lock, 0, "the lock can be lifted again");
}

/// SetLimit restarts the period counter in the new period unit (daily -> weekly would otherwise
/// leave period_index in days, so every weekly Spend is StalePeriod); spend.nonce is kept.
#[test]
fn set_limit_resets_the_period_counter_but_not_the_spend_nonce() {
    let (v, ms) = limited();
    let day = 20_000; // today's index in PERIOD units
    let a = post(run(v, &ms, &spend(v, &ms, 60, day, &[1])).unwrap());
    let weekly = Action::Config(ConfigOp::SetLimit(Some(Limit {
        amount: 500,
        period: 7 * PERIOD,
        threshold: 1,
    })));
    let b = post(run(v, &a, &execute(v, &a, weekly, &[1, 2])).unwrap());
    assert_eq!(b.spend, SpendState { period_index: 0, spent: 0, nonce: 1 });
    let week = day / 7;
    let c = post(run(v, &b, &spend(v, &b, 400, week, &[1])).unwrap());
    assert_eq!(c.spend, SpendState { period_index: week, spent: 400, nonce: 2 });
    assert_eq!(run(v, &c, &spend(v, &c, 1, week - 1, &[1])).err(), Some(Error::StalePeriod));
    assert_eq!(run(v, &c, &spend(v, &c, 101, week, &[1])).err(), Some(Error::OverLimit));
}

/// Approvers do not sign t, so only the proposer may queue (and so choose t): another INITIATE
/// member who saw the approvals cannot front-run with t = expiry - time_lock - 1.
#[test]
fn only_the_proposer_queues_and_t_is_bound_by_the_queuer_signature() {
    let (v, ms) = timelocked();
    let late = 1_000 - TL - 1;
    assert_eq!(
        run(v, &ms, &queue_by(v, &ms, transfer(v, 5), 1_000, late, &[1, 2], 1)).err(),
        Some(Error::QueuerNotProposer)
    );
    // the proposer's own Queue resubmitted with another t than it signed
    let Instruction::Queue { action, expiry, proposer, approvals, queuer, .. } =
        queue(v, &ms, transfer(v, 5), 500)
    else {
        unreachable!()
    };
    let tampered = Instruction::Queue { action, expiry, t: late, proposer, approvals, queuer };
    assert_eq!(run(v, &ms, &tampered).err(), Some(Error::BadSignature));
    assert!(run(v, &ms, &queue(v, &ms, transfer(v, 5), 500)).is_ok());
}
