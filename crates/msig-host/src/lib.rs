//! Host side of the multisig: the deployed program account `P`, dev keys and signed `Execute`
//! builders, and an in-process LEZ harness (`V03State`, no sequencer) for private transactions.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use k256::schnorr::SigningKey;
use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use key_protocol::key_management::secret_holders::PrivateKeyHolder;
use lee::error::LeeError;
use lee::privacy_preserving_transaction::circuit::ProgramCatalog;
use lee::privacy_preserving_transaction::{Message as PpMessage, WitnessSet as PpWitnessSet};
use lee::program::Program;
use lee::{
    AccountId, Actor, PrivacyPreservingTransaction, ProvingInput, PublicExecutionContext, RootCall,
    SenderPresentation, Simulation, TransactionEntry, V03State, ValidatedStateDiff,
};
use lee_core::account::{Account, Nonce};
use lee_core::native_token::{Message as Native, NATIVE_TOKEN_PROGRAM_ID};
use lee_core::program::{
    PROGRAM_LOADER_ACCOUNT_ID, PdaSeed, ProgramHeader, immutable_mirror_commitment,
};
use lee::test_utils::TestPrivateKeys;
use lee_core::{
    AuthorizationSecretKey, Commitment, FeeTransfer, MembershipProof, Nullifier, NullifierWitness,
    PrivateWitness,
    RegularKey, WitnessKind,
};
use msig_core::*;
use sha2::{Digest, Sha256};

pub use msig_methods::{MSIG_ELF, MSIG_ID, NOOP_ELF, NOOP_ID};

/// Frozen image v1 (history): Create/Execute/Config/Cancel.
pub const IMAGE_ID_V1: &str = "9170f3e9b712f5ff2bcf7b59eb74d3f2d4cb219256ec1418ff492f9497204e1d";
/// Frozen image v2 (history): adds Queue/ExecuteQueued and Spend.
pub const IMAGE_ID_V2: &str = "6011b7d8b42510101d329b9a423334b2e36381e8de4d4e31e66d0f1c8f15dc67";
/// Frozen image v2.1 (evidence/IMAGE-ID-v2.1.md): v2 plus three fixes (time lock cap, SetLimit
/// resets the period counter, only the proposer queues); the benchmark binaries assert it.
pub const IMAGE_ID_V2_1: &str = "e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557";

/// Hex of the image id as `evidence/IMAGE-ID-v2.1.md` prints it.
pub fn msig_image_hex() -> String {
    hex::encode(risc0_zkvm::sha::Digest::from(MSIG_ID))
}

/// Deploy key of the multisig program header account, from its image id:
/// `sha256("rfp005/msig/deploy-key" ‖ image_id as LE words)`.
/// shortcut: the key is public by derivation, so anyone can deploy
/// first at that address on a shared network; use a private deploy key for a public testnet.
pub fn deploy_key(image_id: [u32; 8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rfp005/msig/deploy-key");
    for w in image_id {
        h.update(w.to_le_bytes());
    }
    h.finalize().into()
}

pub fn key_account(k: [u8; 32]) -> AccountId {
    AccountId::from(&lee::PublicKey::new_from_private_key(
        &lee::PrivateKey::try_new(k).expect("valid key"),
    ))
}

/// `P`: the deployed program account (header), not `from_builtin_program(MSIG_ID)`.
pub fn program_account() -> AccountId {
    key_account(deploy_key(MSIG_ID))
}

pub fn catalog() -> ProgramCatalog {
    ProgramCatalog::from([(program_account(), Program::new(MSIG_ELF.into()).unwrap())])
}

/// `P`'s program claimed as `Undisclosed`: the tx carries only a commitment-set root that
/// contains the immutable header's mirror commitment, so it names neither `P` nor the image id.
/// `header` is the deployed header account's `ProgramHeader` (must be immutable); `proof` is the
/// membership proof of `immutable_mirror_commitment(P, &header)`.
pub fn catalog_undisclosed(header: ProgramHeader, proof: MembershipProof) -> ProgramCatalog {
    catalog().with_undisclosed(program_account(), header, proof)
}

/// The mirror commitment the sequencer holds for the immutable header at `P`.
pub fn mirror_commitment(header: &ProgramHeader) -> Commitment {
    immutable_mirror_commitment(program_account(), header)
}

/// Dev member key `i` (deterministic, local only; never a real key).
pub fn dev_key(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[0x10 + i; 32]).expect("valid dev key")
}

/// `n` dev members, all with every permission (INITIATE|VOTE|EXECUTE|SPEND), threshold `m`.
pub fn dev_cfg(n: u8, m: u8) -> InitConfig {
    InitConfig {
        chain_tag: Sha256::digest(b"rfp005/local-dev").into(),
        members: (0..n)
            .map(|i| Member {
                key: dev_key(i).verifying_key().to_bytes().into(),
                perms: ALL_PERMS,
            })
            .collect(),
        threshold: m,
    }
}

pub fn initial_state(cfg: &InitConfig, salt: &[u8; 32]) -> Multisig {
    Multisig::new(cfg, seed(cfg, salt))
}

/// A native transfer out of `v` under the vault seed.
pub fn native_transfer(v: AccountId, to: AccountId, amount: u128) -> Action {
    Action::Call {
        to: Actor::new(v, NATIVE_TOKEN_PROGRAM_ID),
        data: borsh::to_vec(&Native::Transfer { to, amount }).unwrap(),
        with_authority: true,
    }
}

fn sign(i: u8, d: &[u8; 32]) -> Sig {
    Sig {
        index: i,
        sig: dev_key(i).sign_raw(d, &[0; 32]).unwrap().to_bytes(),
    }
}

/// `Execute` signed by dev member 0 as proposer and executor and `approvers` (ascending) as voters.
pub fn signed_execute(
    v: AccountId,
    ms: &Multisig,
    action: Action,
    expiry: u64,
    approvers: &[u8],
) -> Instruction {
    let [dp, da, de] = execute_digests(&v, ms, &action, expiry);
    Instruction::Execute {
        action,
        expiry,
        proposer: sign(0, &dp),
        approvals: approvers.iter().map(|&i| sign(i, &da)).collect(),
        executor: sign(0, &de),
    }
}

/// `Queue` at time `t`, signed like `signed_execute`, with dev member 0 as queuer.
pub fn signed_queue(
    v: AccountId,
    ms: &Multisig,
    action: Action,
    expiry: u64,
    t: u64,
    approvers: &[u8],
) -> Instruction {
    let [dp, da, dq, _] = digests(&v, ms, &action, expiry, t);
    Instruction::Queue {
        action,
        expiry,
        t,
        proposer: sign(0, &dp),
        approvals: approvers.iter().map(|&i| sign(i, &da)).collect(),
        queuer: sign(0, &dq),
    }
}

/// `ExecuteQueued` of `action` (the one in `ms.queued`), dev member 0 as executor.
pub fn signed_execute_queued(v: AccountId, ms: &Multisig, action: Action) -> Instruction {
    let q = ms.queued.as_ref().expect("an action is queued");
    let [.., de] = digests(&v, ms, &action, q.expiry, q.t);
    Instruction::ExecuteQueued {
        action,
        executor: sign(0, &de),
    }
}

/// `Spend` of `amount` to `to` in period `p`, signed by dev members `signers` (ascending).
pub fn signed_spend(
    v: AccountId,
    ms: &Multisig,
    to: AccountId,
    amount: u128,
    p: u64,
    signers: &[u8],
) -> Instruction {
    let d = spend_digest(&v, ms, &to, amount, p);
    Instruction::Spend {
        to,
        amount,
        p,
        signers: signers.iter().map(|&i| sign(i, &d)).collect(),
    }
}

/// The private PDA `V` of a GMS for `seed`: keys from `derive_keys_for_pda(MSIG_ID, seed)`,
/// address bound to `P`.
pub fn private_vault(gms: &GroupKeyHolder, seed: [u8; 32]) -> (AccountId, PrivateKeyHolder) {
    let keys = gms.derive_keys_for_pda(&MSIG_ID, &PdaSeed::new(seed));
    let v = AccountId::for_private_pda(
        &program_account(),
        &PdaSeed::new(seed),
        &keys.generate_nullifier_public_key(),
        &keys.generate_viewing_public_key(),
    );
    (v, keys)
}

/// `Create` for the private posture.
pub fn create_private(cfg: InitConfig, salt: [u8; 32], keys: &PrivateKeyHolder) -> Instruction {
    Instruction::Create {
        cfg,
        salt,
        keys: Some((
            keys.generate_nullifier_public_key(),
            keys.generate_viewing_public_key(),
        )),
    }
}

/// A submitter's private fee payer (regular private account, `FeeOnly` witness) and the public
/// account the fee is credited to. `refund_to` is always `None` (§3.3 item 4).
pub struct FeePayer {
    pub keys: TestPrivateKeys,
    pub account: Account,
    pub inbox: AccountId,
}

impl FeePayer {
    pub fn id(&self) -> AccountId {
        self.keys.account_id()
    }

    pub fn transfer(&self, amount: u128) -> FeeTransfer {
        FeeTransfer {
            payer: self.id(),
            recipient: self.inbox,
            amount,
            refund_to: None,
        }
    }
}

/// In-process LEZ state with the multisig deployed at `P`.
pub struct Chain {
    pub state: V03State,
    pub block: u64,
    pub timestamp_ms: u64,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            state: V03State::new()
                .with_named_programs([(program_account(), Program::new(MSIG_ELF.into()).unwrap())]),
            block: 1,
            timestamp_ms: 1_000_000,
        }
    }
}

/// Max cycles of a public part (`MAX_GAS_EXEC`, lez/programs/fee/core/src/market.rs).
pub const MAX_GAS_EXEC: u64 = 10_000_000;

impl Chain {
    /// Test-only seeding: `V` exists privately with `ms` as its `(V, P)` state and `balance`
    /// native. shortcut: writes the commitment directly instead of a Create + fund tx pair; the
    /// live e2e (scripts/e2e-private.sh) runs the real Create and fund path.
    pub fn seed_vault(
        &mut self,
        v: AccountId,
        keys: &PrivateKeyHolder,
        ms: &Multisig,
        balance: u128,
    ) -> Account {
        let acct = Account {
            nonce: Nonce(7),
            ..Account::funded(balance)
                .with_actor_state(program_account(), borsh::to_vec(ms).unwrap().into())
        };
        let nf = Nullifier::for_account_initialization(&v, &keys.nullifier_secret_key());
        let state = std::mem::take(&mut self.state);
        self.state = state.with_private_accounts([(Commitment::new(&v, &acct), nf)]);
        acct
    }

    /// An existing public account (a public actor must exist or carry admission evidence).
    pub fn open_public(&mut self, id: AccountId, balance: u128) {
        let state = std::mem::take(&mut self.state);
        self.state = state.with_public_account_balances([(id, balance)]);
    }

    /// Test-only seeding of a funded private fee payer (dev keys) and an open fee inbox.
    pub fn seed_fee_payer(&mut self, balance: u128) -> FeePayer {
        let keys = TestPrivateKeys {
            ask: AuthorizationSecretKey([0x61; 32]),
            d: [0x62; 32],
            z: [0x63; 32],
        };
        let account = Account::funded(balance);
        let id = keys.account_id();
        let nf = Nullifier::for_account_initialization(&id, &keys.nsk());
        let inbox = AccountId::new([0xfe; 32]);
        let state = std::mem::take(&mut self.state);
        self.state = state
            .with_private_accounts([(Commitment::new(&id, &account), nf)])
            .with_public_account_balances([(inbox, 1)]);
        FeePayer {
            keys,
            account,
            inbox,
        }
    }

    pub fn fee_witness(&self, fp: &FeePayer) -> PrivateWitness {
        PrivateWitness {
            vpk: fp.keys.vpk(),
            random_seed: [4; 32],
            kind: WitnessKind::Regular(RegularKey::FeeOnly(fp.keys.ask)),
            nullifier: NullifierWitness::Update {
                membership_proof: self
                    .state
                    .get_proof_for_commitment(&Commitment::new(&fp.id(), &fp.account))
                    .expect("fee payer commitment in state"),
                account: fp.account.clone(),
            },
            address_openings: BTreeSet::new(),
        }
    }

    pub fn witness(
        &self,
        v: AccountId,
        keys: &PrivateKeyHolder,
        seed: [u8; 32],
        pre: Option<Account>,
    ) -> PrivateWitness {
        let nullifier = match pre {
            None => NullifierWitness::Init {
                commitment_root: self.state.commitment_root(),
            },
            Some(account) => NullifierWitness::Update {
                membership_proof: self
                    .state
                    .get_proof_for_commitment(&Commitment::new(&v, &account))
                    .expect("vault commitment in state"),
                account,
            },
        };
        PrivateWitness {
            vpk: keys.generate_viewing_public_key(),
            random_seed: [3; 32],
            kind: WitnessKind::Pda {
                nsk: keys.nullifier_secret_key(),
                binding: (program_account(), PdaSeed::new(seed)),
            },
            nullifier,
            address_openings: BTreeSet::new(),
        }
    }

    /// `lee::execute_and_prove` of a private root call (real proof unless `RISC0_DEV_MODE`),
    /// timed around the call alone.
    pub fn prove(
        &self,
        witness: PrivateWitness,
        to: Actor,
        msg: &Instruction,
        public: Vec<Actor>,
    ) -> Result<(PrivacyPreservingTransaction, Duration), LeeError> {
        self.prove_raw(witness, to, borsh::to_vec(msg).unwrap(), public)
    }

    pub fn prove_raw(
        &self,
        witness: PrivateWitness,
        to: Actor,
        message: Vec<u8>,
        public: Vec<Actor>,
    ) -> Result<(PrivacyPreservingTransaction, Duration), LeeError> {
        self.prove_tx(vec![witness], to, message, public, None, &self.msig_catalog())
    }

    /// The `Undisclosed` catalog for this chain's deployed (immutable) header at `P`.
    pub fn msig_catalog(&self) -> ProgramCatalog {
        let header = ProgramHeader::from_bytes(
            self.state
                .get_account_by_id(program_account())
                .data
                .actor_state(PROGRAM_LOADER_ACCOUNT_ID),
        )
        .expect("header at P");
        let proof = self
            .state
            .get_proof_for_commitment(&mirror_commitment(&header))
            .expect("immutable header mirrored into the commitment set");
        catalog_undisclosed(header, proof)
    }

    /// The general form: several private witnesses, an optional fee transfer (its payer's witness
    /// in `witnesses`, its inbox in `public`), any program catalog. Every note is padded to
    /// `NOTE_PAD`.
    pub fn prove_tx(
        &self,
        witnesses: Vec<PrivateWitness>,
        to: Actor,
        message: Vec<u8>,
        public: Vec<Actor>,
        fee: Option<FeeTransfer>,
        programs: &ProgramCatalog,
    ) -> Result<(PrivacyPreservingTransaction, Duration), LeeError> {
        let states: HashMap<Actor, lee::ActorState> = public
            .iter()
            .map(|a| {
                (
                    *a,
                    self.state
                        .get_account_by_id(a.account_id)
                        .data
                        .actor_state(a.program_account_id)
                        .clone(),
                )
            })
            .collect();
        let input = ProvingInput {
            claims: Vec::new(),
            root: TransactionEntry::Sync(RootCall { to, message }),
            public_context: PublicExecutionContext::new(public, []),
            private_witnesses: witnesses,
            dummy_inputs: Vec::new(),
            ciphertext_padding: Some(NOTE_PAD),
            recoveries: Vec::new(),
            private_promotions: BTreeSet::new(),
            fee,
        };
        let t = Instant::now();
        let (out, proof) = lee::execute_and_prove(
            input,
            &Simulation {
                public_actor_states: states,
                admitted_accounts: None,
            },
            programs,
            |_| SenderPresentation::Blinded([9; 32]),
            |_, _| false,
            |b| Err(LeeError::InvalidInput(format!("no seal for {:?}", b.to))),
        )?;
        let took = t.elapsed();
        let message = PpMessage::from_circuit_output(BTreeMap::new(), out);
        let ws = PpWitnessSet::for_message(&message, proof, &[]);
        Ok((PrivacyPreservingTransaction::new(message, ws), took))
    }

    /// Validate (proof included unless dev mode) and apply a private tx.
    pub fn settle(&mut self, tx: &PrivacyPreservingTransaction) -> Result<(), LeeError> {
        self.block += 1;
        let (diff, _) = ValidatedStateDiff::from_privacy_preserving_transaction_metered(
            tx,
            &self.state,
            self.block,
            self.timestamp_ms,
            MAX_GAS_EXEC,
        )?;
        self.state.apply_state_diff(diff)?;
        Ok(())
    }

    pub fn has(&self, v: AccountId, acct: &Account) -> bool {
        self.state
            .get_proof_for_commitment(&Commitment::new(&v, acct))
            .is_some()
    }

    pub fn public_balance(&self, id: AccountId) -> u128 {
        self.state
            .get_account_by_id(id)
            .data
            .native_balance()
            .unwrap()
    }
}

/// `LeeError` of a failed program run → the multisig error, if the guest exited with one of its
/// codes.
pub fn msig_error(e: &LeeError) -> Option<Error> {
    match e {
        LeeError::ProgramExitedWithCode { code, .. } => Error::from_code(*code),
        _ => None,
    }
}
