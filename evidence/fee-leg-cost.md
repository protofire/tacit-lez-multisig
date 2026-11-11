# Cost of the v0.4 fee leg in the LEZ privacy circuit (v0.3.0 vs dev vs v0.4.0-rc1)

Executor cycle counts only (`RISC0_DEV_MODE=1`, no real proofs), risc0-zkvm 3.0.5 on all revisions,
segment po2 = 20 (default). "Padded" = risc0 `total cycles` (sum of 2^po2 over segments).

## Result

The circuit itself did not get heavier. The same private transfer, with fees left out, costs the same
within 1 % on v0.3.0, the pre-0.4 dev revision 9f004c6b and v0.4.0-rc1. What changed in rc1 is the
wallet's transaction shape. Since LEZ PR #985 ("pptx fees with an in-proof reserve and refund", merged as
1806e87 = v0.4.0-rc1) every wallet private transaction carries an in-proof fee transfer and a fee recovery
binding:

- the fee leg proves a fallback encrypted note for the payer: about 0.93M user cycles (+933,884);
- the recovery binding adds one more ML-KEM-768 encapsulation and keystream: about 0.86M (+858,995).

Together that is about +1.8M user cycles on a wallet-shaped 7-slot private→private transfer:
1,948,086 → 3,740,965 user cycles, 3 → 5 segments, 2.36M → 4.33M padded cycles. Neither leg exists on
v0.3.0 or 9f004c6b.

The `fund` transaction in `tx-cycles.md` (4,858,960 user cycles, 6 segments, ~5.77M padded) is this rc1
shape plus a PDA recipient, a separate fee-only payer and real-depth membership paths; that remainder was
not broken down.

Revisions: v0.3.0 tag = db66590a; v0.4.0-rc1 tag = 1806e87; dev 9f004c6b (dev 7e3f7f76 ships the same
circuit binary, blob a863e67). The transaction always pads to 7 private slots: v0.3.0
`MAX_PRIVATE_ACCOUNTS = 7` (lez/wallet/src/account_manager.rs:305), dev and rc1
`PADDED_PRIVATE_ACTIONS = 7` (:380 / :387). Ciphertext pad 512 on all three.

## Table (outer privacy circuit, one executor session per row)

| case | private actions | rev | user cycles | segments | padded cycles |
|---|---|---|---|---|---|
| public → private native transfer, no padding (upstream test) | 1 | v0.3.0 | 924,282 | 2 | 1,114,112 |
| | 1 | 9f004c6b | 930,758 | 2 | 1,179,648 |
| | 1 | rc1 | 934,221 | 2 | 1,179,648 |
| private → private native transfer, no padding (upstream test) | 2 | v0.3.0 | 1,806,833 | 2 | 2,097,152 |
| | 2 | 9f004c6b | 1,820,097 | 3 | 2,129,920 |
| | 2 | rc1 | 1,822,558 | 3 | 2,162,688 |
| private → private, pad 512, 0 dummies | 2 | v0.3.0 | 1,866,503 | 3 | 2,162,688 |
| | 2 | 9f004c6b | 1,881,897 | 3 | 2,228,224 |
| | 2 | rc1 | 1,884,366 | 3 | 2,228,224 |
| **wallet shape without fees**: pad 512 + 5 dummies | 7 | v0.3.0 | 1,925,292 | 3 | 2,359,296 |
| | 7 | 9f004c6b | 1,939,078 | 3 | 2,359,296 |
| | 7 | rc1 | 1,948,086 | 3 | 2,359,296 |
| + in-proof fee (sender pays, fallback note) | 7 | rc1 only | 2,881,970 | 4 | 3,407,872 |
| **rc1 wallet shape**: + fee + fee recovery binding | 7 | rc1 only | 3,740,965 | 5 | 4,325,376 |

Unit costs from the rows above:
- one real private action (ML-KEM encapsulation, note encryption, commitment, nullifier): ~0.88–0.93M user cycles;
- one dummy slot: ~12–13K user cycles (two seeded hashes plus a 512-byte copy); 5 dummies cost ~60K, so the 7-slot padding is cheap;
- rc1 fee leg: +933,884 user cycles; rc1 fee recovery binding: +858,995.

## Code changes that explain it

v0.3.0 → 9f004c6b: an execution-model rewrite with no per-action cost change.
- The plan/apply `Backend` + `ProvenCall` transcript is replaced by `ExecutionEnvironment::handle_message` with one
  `env::verify` per non-native transition (lee/privacy_preserving_circuit/src/private_backend.rs, whole file).
  The native token is still recomputed in-circuit with no inner proof in both.
- `PrivatePart::new(...).execute` replaces `ExecutionState::initialize(...).run` (src/main.rs:75-102 @9f004c6b).
- New journal fields: boundary, sealed async messages, `recovery_bindings`, `public_root`, async message spend
  (src/output.rs:31-55 @9f004c6b). They are empty for a plain transfer, so they cost nothing there.
- Per-action crypto unchanged: `SharedSecretKey::encapsulate_deterministic` (ML-KEM-768) in both
  (output.rs:177 @v0.3.0 `emit_private_output`; output.rs:158 @9f004c6b `private_action`). Dummies are now
  asserted to carry an ML-KEM-length epk (output.rs `emit_dummy_output`), which is negligible.
- Net: +0.7–0.8 % user cycles.

9f004c6b → v0.4.0-rc1 (PR #985, fees): the circuit is unchanged when `fee = None` (+0.2–0.5 %).
- `ProvingInput` gains `claims` and `fee: Option<FeeTransfer>` (lee/state_machine/core/src/circuit_io.rs:186-188 @rc1).
- With a fee, the circuit runs the fee transfer as an extra entry and builds `fee_fallback`: a second full
  `private_action` for the payer (its post-state after the fee alone), encrypted under its own esk
  (lee/privacy_preserving_circuit/src/output.rs:153-190, `private_action` :199, encapsulation :227 @rc1).
  This is the +0.93M.
- Wallet: every private transaction now goes through the in-proof fee path (lez/wallet/src/lib.rs:1625,
  `priced.fee = Some` :1736) and pushes `fee_recovery(payer, opening)` into `recoveries` (lib.rs:1695).
  In-circuit each recovery is `bind_recovery` = one more ML-KEM encapsulation + keystream
  (lee/state_machine/core/src/recovery.rs:88, output.rs:52-54 @rc1). This is the +0.86M.
- Earlier revisions had no fee concept in the wallet (no `FeeTransfer` / `private_fee_payer` in lez/wallet/src at
  db66590a or 9f004c6b). Recoveries existed at 9f004c6b, but only for async-delivery recipients.

## Method and commands

Upstream clone of https://github.com/logos-blockchain/logos-execution-zone, one
`git worktree add --detach wt-<rev> <rev>` per revision. The outer circuit is the committed prebuilt
`artifacts/lee/privacy_preserving_circuit/privacy_preserving_circuit.bin` of each revision (blobs 853ccdb /
a863e67 / d8c9cc1), embedded by `lee/state_machine/build.rs`. No guest build and no Docker were needed
(`RISC0_SKIP_BUILD=1` only skips the unused test guests).

Harness: a small test added to the upstream circuit tests in
`lee/state_machine/src/privacy_preserving_transaction/circuit/tests.rs` (plus `tracing-subscriber` as a lee dev-dep):
- `zz_cycles_bare` calls the upstream tests `prove_privacy_preserving_execution_circuit_public_and_private_accounts`
  and `..._fully_private` unchanged.
- `zz_cycles_wallet` runs the fully-private transfer with `ciphertext_padding: Some(512)` and wallet-shaped dummies
  (512-byte ciphertext, real ML-KEM epk, as `cycle_bench` and the wallet build them). On rc1 it also runs
  `fee: Some(FeeTransfer{payer: sender, recipient: inbox, ..})` with the inbox declared as a public actor
  (as lib.rs:1625), and one payer `RecipientEncryption` (as `fee_recovery`).

```sh
# per revision; each revision needs its own target dir (see caveats)
export CARGO_TARGET_DIR=$D/target-<rev> RISC0_SKIP_BUILD=1 \
  RECURSION_SRC_PATH=<complete recursion_zkr.zip> \
  RISC0_DEV_MODE=1 RISC0_INFO=1 RUST_LOG=risc0_zkvm=info
cargo test --release -p lee --features prove --lib \
  --config 'patch.crates-io.risc0-sys.path="'$D'/risc0-sys-fix"' \
  -- --exact privacy_preserving_transaction::circuit::tests::zz_cycles_wallet --nocapture
git checkout -- Cargo.lock
```
`RECURSION_SRC_PATH` sha256 = 744b999f…8849, the hash risc0-circuit-recursion 4.0.4/4.0.5 expects. All three
revisions lock risc0-zkvm 3.0.5, risc0-circuit-recursion 4.0.4, risc0-sys 1.5.0, toolchain 1.98.1.
`risc0-sys-fix` is the same `cells[i] = Fp(0)` copy that `scripts/bench-mac.sh` builds.

## Caveats

- A shared cargo target dir reuses the previous revision's `lee` build-script output (same fingerprint) and
  embeds the old circuit, which fails with "circuit input must be valid borsh". Use a separate target dir per
  revision and check the `include_bytes!` path in mod.rs.
- These are harness-built inputs, not the real `send_tx` flow. The membership proofs are shallow (a 2-leaf
  set), so real Merkle depth is not exercised. The `fund` case was not reproduced on older revisions, because
  its wallet path (fee payer, PDA claim) does not exist there.
- Not measured: `cycle_bench --ppe` scripted sweeps, and dev 7e3f7f76 separately (same circuit blob as 9f004c6b).
