# Architecture: private M-of-N multisig on LEZ, image v2.1

This document describes what is built at guest image v2.1 and what is planned, section by section. Code wins over this text; every claim cites a source line, a test or a committed evidence file.

**Status tags.**
- **BUILT**: code is in this repository and a named test or evidence file exercises it.
- **PLANNED (Mx / Bx)**: not built. The label names the milestone in `proposal/MILESTONES.md` that builds it.
- **NOT MET**: a requirement that this repository does not meet and no milestone can meet alone.

**Prototype limits.** Member keys are dev keys or plain hex files. Every committed end-to-end transcript uses dev-mode proofs (`RISC0_DEV_MODE=1`). Real proofs appear only in the benchmarks (`docs/BENCHMARKS.md`, `evidence/bench/`). The sequencer is a local standalone LEZ sequencer. The code is not audited and is not for real funds.

| Pin | Value | Source |
|---|---|---|
| LEZ | v0.4.0-rc1, commit `1806e87`, vendored pristine plus 2 wallet patches | `vendor/PATCHES.md` |
| Guest image v2.1 (`MSIG_ID`) | `e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557` | `evidence/IMAGE-ID-v2.1.md`; `crates/msig-host/src/lib.rs:41` |
| Program account `P` (image v2.1) | `2yigtJM3BuZ5TPsVKtB7DU1VewL7rn7R6Sz8DsrVBf7a` | derived, `crates/msig-host/src/lib.rs:52-70` |
| State layout | `VERSION = 2`; a v1 state is rejected, not migrated | `crates/msig-core/src/lib.rs:18`, `:422-428` |
| Members | `MAX_MEMBERS = 30` | `crates/msig-core/src/lib.rs:20` |
| Time lock cap | `MAX_TIME_LOCK` = 30 days in ms | `crates/msig-core/src/lib.rs:23` |
| Note pad | `NOTE_PAD = 4096` | `crates/msig-core/src/lib.rs:41` |
| zkVM | risc0-zkvm 3.0.5, guest built in the pinned Docker builder | `evidence/IMAGE-ID-v2.1.md` |

## 1. Summary and components

- **One program, one account per multisig.** The multisig is a PDA `V` of the program at `P`. Config and counters are the actor state `(V, P)`. The native balance is the actor state `(V, native-token)`.
- **Posture is the account type.** Private posture: a private PDA whose keys come from a Group Master Secret (GMS). Public posture: a public PDA. The program code is the same for both.
- **The quorum lives in the program.** Approvals are BIP-340 signatures collected off-chain. The guest verifies them once per action. A private PDA is never authorised by its key (`vendor/lez/lee/state_machine/core/src/circuit_io.rs:429-431`), so holding the GMS does not let anyone spend.
- **Replay and staleness.** Every signed digest binds `V`, the config epoch, the nonce, the expiry and the action hash.
- **Biggest open risk:** any GMS holder can freeze the vault (section 3.1). There is a decision gate for it in M1.

| Component | Role | Status |
|---|---|---|
| `crates/msig-core` | State, instructions, digests, `validate`, the pure handler `handle` shared by the guest and the tests | BUILT |
| `crates/msig-methods` (`msig-guest`) | Guest binaries `msig` and `noop`. `noop` is a test-only program X for the veto test and is never deployed | BUILT |
| `crates/msig-host` | Transaction builders, in-process chain harness, binaries `msig-e2e`, `msig-bench`, `msig-cycles`, `msig-prove` | BUILT |
| `crates/msig-cli` (`msig`) | Operator CLI over the patched LEZ wallet; signed envelopes as files; decoded signing | BUILT. Arbitrary `Call` and `cancel` are not built: PLANNED (M2) |
| `modules/multisig_module` | Logos module with two calls, `create` and `execute` | BUILT (2 calls). The full API is PLANNED (M3) |
| `modules/headless` | `logosctl` wrapper and retry scenarios | BUILT for `create` and `execute`. Other operations are PLANNED (M3) |
| `idl/msig.idl.json` | SPEL-format JSON generated from the Rust source | BUILT. It passes a schema check only (section 5.3) |
| Coordination room | Envelopes over a Logos chat-module group | Spike only (section 4). PLANNED (M3) |
| QML mini-app | Proposal list beside the room | HTML mock `docs/ui/mini-app.html`. PLANNED (B4) |

What the CLI and the module cover today:

| Operation | Program | `msig` CLI | Module / headless |
|---|---|---|---|
| Create | BUILT | BUILT | BUILT |
| Execute a native transfer | BUILT | BUILT | BUILT |
| Execute an arbitrary `Call` | BUILT (program) | refused, `crates/msig-cli/src/main.rs:721-726`; PLANNED (M2) | PLANNED (M3) |
| Config: add, remove, threshold, time lock, limit | BUILT | BUILT (`SetLimit(None)` has no command) | PLANNED (M3) |
| Cancel | BUILT | no command; PLANNED (M2) | PLANNED (M3) |
| Queue, ExecuteQueued | BUILT | BUILT | PLANNED (M3) |
| Spend | BUILT | BUILT | PLANNED (M3) |
| Propose, approve, reject as separate steps | off-chain | BUILT (envelope files) | PLANNED (M3) |

The CLI and the module both drive the LEZ wallet directly. Only the module runs under the Logos core headless host (`logosctl`). The RFP asks for a CLI over the Logos headless framework (U.3). That is met for `create` and `execute` only. For the full operation set it is PLANNED (M3), with `msig` becoming a front-end to `multisig_module`.

## 2. Program design

### 2.1 Accounts (BUILT)

- **`P`** is the deployed program header account, `key_account(deploy_key(MSIG_ID))` (`crates/msig-host/src/lib.rs:52-70`). It is not `from_builtin_program(MSIG_ID)`.
  - The deploy key is public by derivation, so on a shared network anyone can deploy first at that address. That is acceptable on a local sequencer only. A private deploy key for a public testnet is PLANNED (M5).
- **`V`** is checked at `Create` against the private derivation `for_private_pda(P, seed, npk, vpk)` or the public one `for_public_pda(P, seed)` (`crates/msig-core/src/lib.rs:510-516`).
  - The private keys come from `GroupKeyHolder::derive_keys_for_pda` (`vendor/lez/lee/key_protocol/src/key_management/group_key_holder.rs:125`).
  - Wallet patch 1 binds the PDA to `P` instead of the builtin address (`vendor/PATCHES.md` §1; test `private_vault_is_bound_to_the_deployed_program_account`).
- **`seed`** is `sha256(pad32("LEZ-MSIG/v1/seed") ‖ sha256(borsh(InitConfig)) ‖ salt)` (`crates/msig-core/src/lib.rs:268-276`). `InitConfig` holds `chain_tag`, the members and the threshold. So `V` commits to the initial config and to `P`, and nobody can initialise it with another member set (test `foreign_config_cannot_init`).
- **The program is deployed immutable.** The CLI refuses a mutable header (`crates/msig-cli/src/main.rs:333`), because the `Undisclosed` program claim (section 3.2) requires one.
- **Fee payer.** In the private posture the fee payer is the submitter's own private account and `refund_to = None` (`crates/msig-host/src/lib.rs:237`). The CLI re-checks `refund_to` on the included transaction (`crates/msig-cli/src/main.rs:309-313`).
- **No other accounts exist.** There is no per-proposal and no per-approval account. LEZ v0.4 has no fungible-token program, so "vault assets" means the native balance.
- **Public posture.** It is BUILT and tested at handler level only: the handler tests use a public PDA. A sequencer end-to-end run in the public posture is PLANNED (B3).

### 2.2 State (BUILT, `VERSION = 2`)

`Multisig` (`crates/msig-core/src/lib.rs:85-98`) holds:
- `version`, `chain_tag`, `seed`;
- `members: Vec<{ key: BIP-340 x-only, perms: u8 }>`;
- `threshold`;
- `time_lock` in ms (0 means immediate);
- `limit: Option<{ amount, period, threshold }>`;
- `config_epoch`, `nonce`;
- `queued: Option<{ digest, t, expiry }>`, a single slot;
- `spend: { period_index, spent, nonce }`.

Permission bits are `INITIATE=1, VOTE=2, EXECUTE=4, SPEND=8` (`crates/msig-core/src/lib.rs:26-30`).

`validate` (`crates/msig-core/src/lib.rs:342-371`) runs at `Create` and after every config op. It requires:
- `1 <= N <= 30`;
- no member with `perms == 0` or with unknown permission bits;
- keys that are valid and unique;
- `1 <= M <= count(VOTE)`;
- at least one `INITIATE` member and one `EXECUTE` member;
- `time_lock <= MAX_TIME_LOCK`;
- if a limit is set: `1 <= limit.threshold <= count(SPEND)` and `period >= 1`.

The architecture draft had a `last` field (digest plus approver bitmap). It is not in the state, so the encrypted state keeps no record of who approved.

### 2.3 Instructions (BUILT)

All instructions are root messages to `Actor(V, P)`.
- A message with `from != None` (a reply) keeps state and does nothing (`crates/msig-core/src/lib.rs:498-500`).
- `is_authorized` is never used for the quorum.
- All time and amount arithmetic is checked.

| Instruction | Checks | Effect; validity window | Code |
|---|---|---|---|
| `Create { cfg, salt, keys }` | `pre_state` empty; `V` matches the derivation; `validate` | epoch 0, nonce 0; no window | `lib.rs:504-520` |
| `Execute { action, expiry, proposer, approvals, executor }` | under a time lock only `Cancel` passes; quorum (2.5) with `t = 0` | `apply`; window `[.., expiry)` | `lib.rs:524-537` |
| `Queue { action, expiry, t, proposer, approvals, queuer }` | `time_lock > 0`; `queuer.index == proposer.index`; `t + time_lock < expiry`; slot empty or its entry expired before `t`; quorum with `t` | `queued = { /queue digest, t, expiry }`; window `[old.expiry, t)` or `[.., t)` | `lib.rs:538-566` |
| `ExecuteQueued { action, executor }` | the recomputed `/queue` digest at the current epoch and nonce equals `queued.digest`; `EXECUTE` signature over `body ‖ t` | `apply`; window `[t + time_lock, expiry)` | `lib.rs:567-581` |
| `Spend { to, amount, p, signers }` | limit set; at least `limit.threshold` `SPEND` signatures; `p >= period_index`; `spent + amount <= limit.amount` | `spend.nonce += 1`; `custody_transfer(V, seed, to, amount)`; window `[p·period, (p+1)·period)` | `lib.rs:582-607` |

`lib.rs` here is `crates/msig-core/src/lib.rs`.

`apply` (`crates/msig-core/src/lib.rs:443-494`) is shared by `Execute` and `ExecuteQueued`. It always sets `nonce += 1` and clears `queued`. Then, per action (`crates/msig-core/src/lib.rs:120-144`):
- **`Call { to, data, with_authority }`** dispatches a `SyncMessage`.
  - The seed grant is attached only when `with_authority` is set and `to.account_id == V`. Otherwise it fails `AuthorityOnForeignAccount`.
  - A call into `P` fails `Reentry`.
  - The grant passes to the target's whole call subtree, so a target called with authority can move `V`'s native balance within that call. The CLI shows `with_authority` before signing. There is no per-program allow-list, because it would check only the first call.
- **`Config(op)`** applies one of `AddMember`, `RemoveMember { key }`, `SetThreshold`, `SetTimeLock`, `SetLimit(Option<Limit>)`, re-runs `validate` and sets `config_epoch += 1`.
  - `SetLimit` also resets `period_index` and `spent`, but keeps the spend nonce.
  - `SetPerms` is not built; it is PLANNED (M2).
  - Removal is by key, because indices shift.
- **`Cancel`** does nothing else. It is the on-chain way to kill an action, and the only action that runs immediately under a time lock.

**Reject** is an off-chain advisory envelope. `execute` counts rejects and ignores them (`crates/msig-cli/src/main.rs:689`). A rejection quorum is not built.

**Error codes** 1 to 22 are in `crates/msig-core/src/lib.rs:197-227`. The guest exits with the code, and the CLI prints its name (`crates/msig-cli/src/main.rs:392-396`).

### 2.4 Digests and signatures (BUILT)

```
D(tag, body) = sha256(pad32(tag) ‖ V ‖ body)                                   lib.rs:289-296
body         = epoch u64le ‖ nonce u64le ‖ expiry u64le ‖ sha256(borsh(Action))  lib.rs:279-286  (56 B)
/propose, /approve           over body                                          lib.rs:300-317
/queue, /execute             over body ‖ t u64le   (t = 0 for an immediate Execute)
/spend                       over epoch ‖ spend.nonce ‖ p ‖ to ‖ amount u128le   lib.rs:331-339  (72 B)
/reject                      over body, client-side only (crates/msig-cli/src/envelope.rs)
tags: "LEZ-MSIG/v1/<name>"                                                       lib.rs:32-37
```

- The scheme is BIP-340 Schnorr over secp256k1, on a 32-byte prehash (`crates/msig-core/src/lib.rs:373-385`). It is the scheme of LEZ accounts.
- Known-answer tests freeze the layouts: `approval_digests_known_answers` and `queue_and_spend_digests_known_answers`.
- `D` omits `chain_tag` and `P`, because `V` already commits to both. Programs see no chain id in `ReceiveInput` (`vendor/lez/lee/state_machine/core/src/program/mod.rs:373-385`). Cross-chain replay protection therefore rests on the creator choosing a distinct `chain_tag`.
- `t` is signed, and `/queue` and `/execute` are distinct tags, so a `Queue` signature cannot execute (test `queue_and_execute_signatures_are_not_interchangeable`).
- **Hardware signing.** The digest is a plain 32-byte BIP-340 prehash, so a hardware signer that supports BIP-340 could produce these signatures. Today member keys are plain hex files. Hardware-wallet (Keycard) signing is planned and not built. No milestone is assigned to it yet.

### 2.5 Quorum rule and roles (BIP-340 checks BUILT; R.1)

`check_quorum` (`crates/msig-core/src/lib.rs:404-420`) verifies:
1. at least `M` approvals under `/approve`, from `VOTE` members, with strictly ascending indices, so no member is counted twice (`verify_set`, `crates/msig-core/src/lib.rs:389-397`);
2. the proposer under `/propose`, from an `INITIATE` member;
3. one stage signature: the executor (`EXECUTE`) under `/execute`, or the queuer (`INITIATE`, and the same member as the proposer) under `/queue`.

That is `M + 2` verifications. `ExecuteQueued` verifies one signature. The digests are recomputed from state and from the submitted action, never taken from the caller. The proposer counts toward `M` only if they also sign `/approve` (test `proposer_does_not_count_as_approver`).

| Role | What the program guarantees |
|---|---|
| Initiate | Nothing runs or is queued unless an `INITIATE` member signed exactly this action under `/propose`. Only the proposer can queue it and choose `t` (`QueuerNotProposer`, `crates/msig-core/src/lib.rs:549-551`). |
| Vote | Only `VOTE` members count toward `M`. |
| Execute | No `Execute` or `ExecuteQueued` lands without an `EXECUTE` member's signature over this action and `t`. |
| Spend | Only `SPEND` members can use the limit. `Spend` bypasses the time lock by design. |

The program binds consent, not the submitter. Anyone holding the signatures can submit; in the private posture "anyone" means a GMS holder.

Tests: `crates/msig-core/tests/handler.rs`, for example `below_threshold_rejected`, `duplicate_signer_rejected`, `non_member_key_under_member_index_is_rejected`, `proposer_without_initiate_rejected`, `non_executor_cannot_execute`, `exhaustive_subset_oracle_n7`.

### 2.6 Nonce, epoch and the stale-proposal cutoff (BUILT, single queue slot; R.2)

**Rule.** A set of signatures is valid for exactly one `(V, config_epoch, nonce, expiry, action)`. It dies when a transaction that changes `nonce` or `config_epoch` is included. That transaction is any `Execute` or `ExecuteQueued` (nonce, `crates/msig-core/src/lib.rs:444`) and any `Config` (epoch, `crates/msig-core/src/lib.rs:489`). **The cutoff for a pending proposal is the inclusion of the first nonce-consuming transaction other than its own.**

- **No time lock.** Approvals are checked against the member set and threshold at the proposal's own `Execute`.
- **With a time lock.** Approvals are checked at `Queue`. `Queue` does not consume the nonce. `ExecuteQueued` recomputes the `/queue` digest from the current epoch and nonce (`crates/msig-core/src/lib.rs:569-572`). While an entry is live, no `Config` can land: under a time lock a `Config` must itself be queued (`crates/msig-core/src/lib.rs:531-533`), and the slot is occupied (`crates/msig-core/src/lib.rs:557`). The only way out is `Cancel`, which runs immediately and clears the slot. So a queued action never runs under a config other than the one that approved it.
- **Spend** signatures bind `config_epoch` and the separate spend nonce. Any `Config` kills them. Actions do not.

| Case | Outcome |
|---|---|
| Member removed (no time lock) | The removal executes at nonce `n`. Every other signature at `n`, and every signature pre-signed for a later nonce at the old epoch, is dead, including those of remaining members. All re-sign at `(e+1, n+1)`. Tests `config_removes_member_and_stale_approvals_are_rejected`, `old_epoch_approvals_rejected`. |
| Member removed during a delay | The removal is queued like any action. If another action holds the slot, the removal waits until that action executes, expires or is cancelled. Once the removal lands, nothing approved under the old set survives. Test `queued_member_removal_executes_and_kills_old_epoch_approvals`. |
| Member added | Same as removal. The new member's signatures count from the new epoch. |
| Threshold raised | Same. No proposal approved under the lower threshold survives. |
| Competing proposals at one nonce | The first executed (or a `Cancel`) kills the rest. Test `cancel_consumes_nonce_and_kills_competing_proposals`. |

**Cost of the single slot.** One delayed action at a time, config changes included.

**Multi-queue** is PLANNED (B1). In that design `Queue` consumes the nonce, entries record their epoch, and an executing `Config` voids the other entries. It is not built, and the rule above is what ships at image v2.1.

**Expiry.** The program sets the validity window `[.., expiry)` (`crates/msig-core/src/lib.rs:536`), and the sequencer enforces it. A handler test asserts the window bound (`quorum_met_executes_and_bumps_nonce`). The CLI refuses an expired envelope before proving (`crates/msig-cli/src/main.rs:664`). No test submits an expired transaction to a sequencer; that test is PLANNED (M2).

### 2.7 Initialisation safety (BUILT, S.7)

- Every instruction except `Create` fails on an empty state (`Uninitialized`).
- `Create` fails on a non-empty state.
- A front-runner cannot initialise `V` with other members, because `V` commits to `cfg`.

Tests: `uninitialised_vault_cannot_be_drained` (in both test files), `foreign_config_cannot_init`, `create_on_initialised_account_fails`, `create_with_wrong_keys_is_rejected`.

## 3. Privacy

### 3.1 What a GMS holder can and cannot do; the liveness veto

The GMS gives every holder the same nullifier secret key and viewing key for `V`. At the key layer that is 1-of-N.

**Cannot spend (BUILT).** Test `gms_holder_cannot_bypass_quorum` (`crates/msig-host/tests/private.rs`) covers these cases:
- (a) a direct native debit of `V` fails with `UnauthorizedSender`;
- (a') a full quorum whose `Call` omits the authority also fails;
- (b) `M-1` approvals fail with `BelowThreshold`;
- (d) a second `Create` fails.

Case (c), a foreign program presenting `V`'s seed, relies on LEZ's own seed-grant check (`private_seed_grant`, `vendor/lez/lee/state_machine/core/src/execution_state.rs:1516`) and its tests. No test of ours covers it.

**Can freeze the vault (BUILT as a reproduced attack; not fixed).**
- Any transaction with a witness for `V` spends `V`'s current nullifier, with no multisig instruction involved.
- Test `gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced`: a GMS holder's root message to `Actor(V, X)`, for a trivial program X, lands. The honest `Execute` proven against the previous commitment is then rejected. Balance and config are unchanged.
- In plain words: **anyone holding the GMS can freeze the vault's funds** for as long as they keep landing such transactions. That includes a removed member and anyone the GMS leaked to. Each attempt costs them one private transaction (a proof and a fee).
- The freeze blocks `Execute`, `Config`, `Queue`, `Spend` and the re-key, since the re-key is itself an `Execute`. It cannot move funds.
- The program cannot prevent it. It is a LEZ-level property (Q12).

**Mitigation path.** None of these is built.
1. **Narrow the holder set** (design analysis, untested). Signing an approval needs only `V`, the epoch, the nonce and the member list. The viewing key alone decrypts those (test `viewing_key_alone_decrypts_state`). Only the member who proves and submits needs the nullifier secret. Giving voters the viewing key and the GMS only to `EXECUTE` members shrinks the veto set from N to the executors. It does not remove the veto, and a removed executor keeps it. It needs a viewing-key-only `join` in the CLI (PLANNED, M2).
2. **A LEZ change**, put to Logos with the test above as the reproducer: a nullifier advance gated by the owning program, a read-only witness, or rotation of the nullifier key of an existing account.
3. **Public posture.** It has no GMS and therefore no veto, but every item in section 3.2 becomes public.

**M1 decision gate.** By the end of M1 we record Logos' answer on item 2 in `docs/DECISIONS.md`.
- **Branch A (a LEZ fix is committed):** the private posture stays the default, and the fix is adopted when available.
- **Branch B (no fix):** the private posture ships with mitigation 1. The freeze risk is shown at `create`. The public posture (B3) becomes the recommended default for treasuries that cannot accept a freeze.

How each branch changes the milestones is stated in `proposal/MILESTONES.md`, M1 gate (b).

**Removed member.** Spending authority is revoked at once: their approvals fail on chain. They keep read access to every later state note and keep the veto until the members re-key into a new vault under a fresh GMS. The re-key flow is PLANNED (M2), and a hostile holder can stall it.

### 3.2 Who sees what (BUILT for `Execute`; PRIVACY.md has the per-item table)

The full per-item table for the RFP's ten Privacy Architecture items, by audience, is in `docs/PRIVACY.md`. These are the mechanisms:

- **Hidden from the transaction** (asserted per field by `private_tx_byte_scan`, N = 3 and N = 30; `evidence/leak-surface.md`): `V`, `P`, the image id, member keys and member account ids, and the fee payer id. `refund_to` is `None`.
  - The program claim is `Undisclosed { root }` (`crates/msig-host/src/lib.rs:80-82`), so the transaction does not name the program.
  - Every note is padded to `NOTE_PAD`, so note length does not reveal N or the config.
- **Published:**
  - that a private transaction exists: its nullifier, commitment and 4096-byte notes;
  - the fee amount and fee inbox;
  - the validity window (the expiry; `t` for `Queue`; the period for `Spend`);
  - the public effects of the action. A native payout to a public account shows its recipient and amount, with native-token as the sender program;
  - the transaction size.
- **The transaction is distinguishable as ours** even though it does not name the program:
  - **Pad class.** Wallet patch 2 pads every note to 4096 bytes instead of the stock 512 (`vendor/lez/lez/wallet/src/account_manager.rs:37`). Every transaction this wallet emits therefore stands out from stock-wallet transactions. Today the anonymity set is "transactions padded to 4096", which in practice means this application.
  - **Size per operation.** Real-proof sizes on image v2.1: `Create` 311,206 B, `Execute` 311,670 B, fund 312,046 B (`evidence/bench/mac-cpu.log`, `docs/BENCHMARKS.md`). Size separates the operation types. It does not separate M: `Execute` is 311,670 B at M = 5, 10 and 30. `Queue`, `ExecuteQueued`, `Spend` and `Config` sizes are not measured; that is PLANNED (M1).
- **Limits of the evidence.** The full byte scan (all needles, every field) ran on dev-mode transactions only. The real-proof benchmark runs check only `P`, the image id, `refund_to`, the note length and the claim type (`docs/BENCHMARKS.md`, Method). A full byte scan on a real-proof transaction is PLANNED (M1).
- **Public posture:** everything is public on chain except pending proposals and room content, which stay off-chain.

### 3.3 Cross-layer correlations left observable (Pr.3)

1. **Timing.** Room traffic bursts precede a private transaction.
2. **Throughput.**
   - Storage gas allows at most 4 private transactions per block: `MAX_GAS_STOR = 1,000,000` over `PRIVATE_GAS_STOR = 224,063` (`vendor/lez/lez/programs/fee/core/src/market.rs:27`, `:56`).
   - By bytes, a 1 MiB `max_block_size` (the value in `vendor/lez/lez/programs/sequencer_stake/core/src/lib.rs:424`) holds 3 transactions of 311,670 B.
   - Real transactions exceed the fee model's `PROOF_BYTES = 223,551` (`market.rs:51`).
3. **Validity windows.** Expiry, `t` and the spend period are public. They reveal the time-lock and period lengths when transactions are linked by timing. The CLI does not round them; expiry is `now + --expiry-secs` (`crates/msig-cli/src/main.rs:428`). Rounding to a grid is PLANNED (M2).
4. **Pad class and size** (section 3.2).
5. **Fee amount** is public. `refund_to` must be `None` and is asserted.
6. **Public effects** of an executed action.
7. **Note length** would leak if state on `V` outgrew the pad. At N = 30 the multisig state is about 1.2 KiB.
8. **The LEZ-to-chat binding** would exist only inside the E2EE room (section 4.2). It is not built.
9. **Transport.** What the chat delivery layer reveals (group id, sizes, IP addresses) is not measured. Measuring it is PLANNED (M3).
10. **Nullifier races.** A failed honest transaction followed by a retry is visible to the sequencer as a pattern.

### 3.4 Selective disclosure (BUILT: decryption test; export command PLANNED, M2)

- **Mechanism.** Give the audience the viewing secret key of `V`, never the GMS and never the nullifier secret. Test `viewing_key_alone_decrypts_state` opens a post-state note with the viewing key only. The note holds the members, threshold, epoch, nonce, time lock, limit and balance.
- **What the audience learns.** The whole state, including every member's BIP-340 key. Granularity is the whole account, read-only. The audience gets no spending power and no veto.
- **Configuration and holdings.** These are covered: every state note decrypts.
- **Activity.** The state keeps no action history: there is no `last` field and no log. The audience can decrypt the post-state note of every past transaction of `V`. In principle that shows a sequence of nonce, epoch, balance and config changes, but not the action payloads beyond their public effects. Only a single note is tested. History reconstruction is PLANNED (M2).
- **Wide audience.** It is the same key shared more widely. There is no per-audience key and no per-field disclosure.
- **Revocation.** Only by re-keying into a new vault, which a GMS holder can stall (section 3.1).
- **Not built.** The wallet has no viewing-key export command (PLANNED, M2). Proof of holding is not offered.

## 4. Coordination room

### 4.1 What is measured: chat-module spikes (no product code)

Two spikes ran, both on preset `logos.dev`, with dev-mode proofs and the chat module v0.3.0 installed from release `.lgx` files by path.
- **Run 2**: 2 members, placeholder payload. Its findings are summarised below; its raw results are not part of this repository.
- **Run 3** (`spikes/chat-module/run3/RESULTS.md`): 3 members, a 2-of-3 vault, real msig envelopes.

What crossed in run 3:
- A proposed in a group, as `msig1:prop:` plus base64 of the CLI's JSON envelope.
- B and C decoded it with `msig show`, approved with their own keys and posted back.
- A dropped a forged approval (B's key over different bytes; `msig show` marks it `INVALID`) and executed. The vault went from 1000 to 700.
- The program's own rejection of a forged signature was not exercised in that run, because the CLI filtered it out first. The handler tests cover the program check.

#### Chat-module findings (measured gaps)

| Gap | Evidence |
|---|---|
| A member offline when a proposal is posted never receives it. After a restart the member has a new chat address, and the old group returns `history_only`. | run 3, step 6 |
| No member removal and no leave call. | chat module docs; design below |
| `send_message` returns success for messages that are dropped (an oversize message over 153,600 B; an RLN-blocked message on `logos.test`). | run 2 |
| `logos.test` needs an RLN module that the catalog does not install. Only `logos.dev` was used. | run 2 |
| Group join takes 62.5 s with 3 members and about 91 s with 2. | runs 3, 2 |
| Catalog install hung for over 7 minutes; installing the release `.lgx` by path works. Packages report `"signature":"unsigned"`. | run 2 |
| One transient key-package registry failure broke `create_conversation`. | run 2 |

### 4.2 Protocol (PLANNED, M3)

- **Envelopes.** The room carries the same signed envelopes as the CLI files (`docs/CLI.md`). The room carries no trust: the program decides validity, and a client signs only a digest it recomputed from bytes it decoded itself.
- **Not built, PLANNED (M3):**
  - **`Bind`:** the member's LEZ key signs `(V, chat address, room id)`. Clients check it against the member list read from chain and reject a `Bind` for another `V`.
  - **`Invite`:** the GMS sealed to the joiner with `seal_for` (`vendor/lez/lee/key_protocol/src/key_management/group_key_holder.rs:172`). Today the vault file is handed over out of band.
  - **A new room per config epoch**, because members cannot be removed.
  - **Re-bind after every restart**, because the chat address changes.
- **Criteria M3 has to meet**, from the gaps above:
  - a member restarts, re-binds under a new address and is re-added;
  - an offline member gets pending proposals on return. The chat module does not deliver to offline members, so the client has to re-post pending envelopes on a re-bind;
  - the room runs on the network that pairs with testnet 0.4.
- **Formal fallback: envelope files.** They are BUILT. Every flow works without a room (`evidence/cli-walkthrough.txt`), and the room only transports the files.

### 4.3 Adversaries

The table is in `docs/ADVERSARY.md`.

## 5. Decoded signing, registry, IDL, packaging

### 5.1 Decoded signing (BUILT in the CLI; GUI PLANNED, B4)

- An envelope carries `signed = V ‖ body ‖ borsh(action)` (`docs/CLI.md`).
- `show`, `approve`, `reject` and `execute` decode the action from those bytes and refuse them if the action hash in `body` does not match (`TAMPERED`). They recompute the digest from the same bytes.
- `approve` prints the decoded action before signing.
- Tests in `crates/msig-cli/src/envelope.rs`: `show_refuses_a_tampered_action`, `show_flags_a_signature_over_other_bytes`, `spend_envelope_signs_the_program_spend_digest`, `new_config_ops_are_decoded`.

### 5.2 Registry (BUILT: local map; F.4)

- The registry is a local JSON map from program account id to name (`--registry`). Two entries are built in: native-token and the multisig program (`crates/msig-cli/src/envelope.rs:280-290`).
- Decoding covers native-token messages and multisig actions only. An unknown program is shown as `UNKNOWN program <id>` with raw hex (test `unknown_program_is_labelled`).
- The registry maps an account, not an image. For this program, `P` derives from the image id through our own deploy-key convention. That does not hold for third-party programs, whose account says nothing about their code.
- **PLANNED:**
  - (M2) register a second program's IDL and decode its call before signing;
  - (M2) an explicit override before signing for an unregistered target;
  - (M3) registry pins signed by k of N members.

### 5.3 IDL (BUILT: SPEL-format JSON with a schema check; U.4 partially met)

- `idl/msig.idl.json` is produced by `scripts/gen_idl.py`, which parses `crates/msig-core/src/lib.rs`. CI checks it for drift.
- `idl/spel-check` deserialises the file with SPEL's own `SpelIdl` structs at a pinned commit. That is a **consistency check against SPEL's schema. It is not SPEL's generator.**
- SPEL's generator and client generator target an older lee_core ABI and were not run.
- The wire format is the borsh enum tag, not SPEL's discriminator (`idl/README.md`).
- Whether SPEL supports LEZ v0.4 actor programs is a question for Logos. The M2 IDL criterion depends on the answer.

### 5.4 Packaging

- **Module (BUILT).** `modules/multisig_module`, package `multisig_module` v0.1.0, with `create` and `execute` (`docs/MODULE-API.md`). The headless run is in `evidence/module-headless-transcript.txt` (image v2.1).
- **Not signed.** The `.lgx` package is not signed (`"signature":"unsigned"` at install, `evidence/module-headless-transcript.txt:21`). A signed package is PLANNED (M4) and is the RFP's main mitigation for U.6.
- **Own catalog:** PLANNED (M4).
- **QML mini-app:** PLANNED (B4).

## 6. Benchmarks and limits (P.1, P.2)

Figures and method: `docs/BENCHMARKS.md`, raw logs in `evidence/bench/`.

- **GPU.** The RTX 3080 run is on image v2.1.
- **Mac.** The Apple M2 Max run is on image v2.1, CPU prover only. risc0 3.0.5 has no Metal proving path (`evidence/bench/mac-NOTE.md`).
- **In-guest cost** is `M + 2` BIP-340 verifications, plus a state decode and encode, plus N key decodes on a `Config`.
- **`MAX_MEMBERS = 30`** comes from the public-posture block rule (`gas_limit <= MAX_GAS_EXEC = 10,000,000`). A worst-case `Config` at M = N = 30 takes about 9.58M guest cycles, and 9.88M at 31 (`crates/msig-core/src/lib.rs:19`). Test `worst_case_quorum_fits_the_block_gas_cap` asserts the fit and prints the counts. No log of that output is committed.
  - At image v2.1 no vault can have more than 30 members.
- **Private posture.** The 10M cap does not apply. The bound is proving time and memory on the submitter's machine. Only the submitter proves; approvers only sign.

## 7. Open questions and assumptions

| # | Question or assumption | Status |
|---|---|---|
| Q12 | Can a private PDA's nullifier advance be gated by its program, or a witness be read-only, or the nullifier key rotated? | Put to Logos; M1 gate (section 3.1) |
| Q10 | Is there a public LEZ v0.4 testnet? | S.4 **NOT MET** until one exists; deployment PLANNED (M5) |
| Q5 | What does the chat delivery layer reveal, and does the chat module run against testnet 0.4? | Unmeasured; PLANNED (M3) |
| Q7 | Will the chat module get member removal and a persistent identity? | Ask Logos; the room-per-epoch and re-bind design works either way |
| Q4 | Is there a chain or zone id a program can bind to? | None in `ReceiveInput`; `chain_tag` in the seed |
| A1 | Single chain (LEZ); no other multisig designs are reused | — |
| A2 | No token program on v0.4, so the limit and the demos use native balance | Revisit when one exists |
| A3 | The deploy key is public by derivation | Private deploy key for a public testnet, PLANNED (M5) |

The requirement-by-requirement mapping is `proposal/COVERAGE.md`. The decision log is `docs/DECISIONS.md`.
