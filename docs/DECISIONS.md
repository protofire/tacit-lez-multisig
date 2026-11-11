# Design decision log (RFP S.14)

Format: context, decision, alternatives, consequence. Status tags as in `docs/ARCHITECTURE.md`: BUILT, PLANNED (milestone), NOT MET. Updated at each milestone. D1-D16 were taken at design time; D17 onward during the build. Where a later entry changes an earlier one, the earlier entry says so. State at image v2.1.

## D1. One program, one account per multisig
- Context: need M-of-N custody with the posture selectable.
- Decision: multisig is a PDA `V` of one program; config, nonce and epoch are actor state under the multisig program, balance under native-token.
- Alternatives: one account per proposal or approval (more on-chain writes, leaks pending metadata).
- Consequence: no per-approval writes. No token program on v0.4, so "assets" means native balance.

## D2. Posture is the account type
- Context: private by default, public option, no second code path.
- Decision: private PDA (keys from a Group Master Secret, GMS) or public PDA; identical program.
- Alternatives: separate public and private programs; mixed postures (not offered).
- Consequence: private posture BUILT on a standalone sequencer; public posture BUILT at handler level, sequencer e2e PLANNED (B3).
## D3. Quorum is enforced in the program, never by the key
- Context: every GMS holder has the same keys for V.
- Decision: authority comes only from the owning program passing the PDA seed; a private PDA is never key-authorized.
- Alternatives: threshold signature on the GMS (not available).
- Consequence: a GMS holder cannot spend (`gms_holder_cannot_bypass_quorum`) but can veto liveness (D4).

## D4. Liveness veto: disclose, gate in M1
- Context: any witness for V spends its nullifier; no read-only mode found. Any GMS holder, including a removed member, can therefore freeze the vault's funds for as long as they keep landing transactions on V.
- Decision: state it plainly as the top risk, reproduce it in a test, put it to Logos with the test as reproducer, and decide at the end of M1. Branch A (LEZ fix committed): private posture stays the default. Branch B (no fix): private posture ships with the holder set narrowed (GMS only to EXECUTE members, viewing key to voters; PLANNED, M2) and the risk shown at create; public posture (B3) becomes the recommended default for treasuries that cannot accept a freeze. ARCHITECTURE §3.1.
- Alternatives: claiming a re-key fixes it (the re-key is itself blockable).
- Consequence: `gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced`. Not fixed.
## D5. Approvals off-chain, BIP-340, verified in-guest once
- Context: no per-approval writes; keep pending payload off chain.
- Decision: BIP-340 Schnorr over secp256k1 on a 32-byte domain-separated SHA-256 digest binding V, epoch, nonce, expiry, action hash. M+2 verifications (proposer, M approvers, executor).
- Alternatives: ECDSA (pattern kept, scheme not); on-chain approvals; dropping the in-guest proposer check (would make Initiate client-side).
- Consequence: cost scales with M; measured in `docs/BENCHMARKS.md`. Digest known answers frozen by `approval_digests_known_answers`.

## D6. Replay and staleness: nonce and epoch
- Context: R.1, R.2.
- Decision: strict nonce bumped by every `Execute` and `ExecuteQueued`; `config_epoch` bumped by every `Config`; all signatures bind both. Cutoff for a pending proposal: inclusion of the first nonce-consuming transaction other than its own (ARCHITECTURE §2.6, with the removal, addition and threshold cases).
- Alternatives: per-proposal ids (needs on-chain state).
- Consequence: competing proposals share a nonce and the first executed kills the rest; any config change forces re-approval of everything pending. Tests: `old_epoch_approvals_rejected`, `replayed_approval_is_rejected`, `queued_member_removal_executes_and_kills_old_epoch_approvals`.
## D7. Roles as permission bits
- Context: F.6.
- Decision: `perms` bits INITIATE, VOTE, EXECUTE, SPEND; the program binds consent, not the submitter.
- Alternatives: weighted votes (not offered).
- Consequence: anyone with the signatures can submit; in the private posture "anyone" is a GMS holder.

## D8. Reject is advisory and off-chain
- Context: avoid on-chain per-vote writes.
- Decision: signed `Reject` envelope; `execute` reports the count and ignores it; only the chain decides.
- Consequence: BUILT in the CLI. A "likely dead" label in the room client is PLANNED (M3); it can be wrong if a member signs both.
## D9. `Call` grants authority to V only; no allow-list
- Context: authority is inherited by the whole call subtree.
- Decision: `with_authority = true` only when `to.account_id == V`; no re-entry into P; no per-program allow-list (it would check only the first call).
- Consequence: signers must be told that a target called with authority can move V's native balance in that call (`authority_only_for_v_and_no_reentry`).

## D10. Fixed note pad 4096
- Context: variable-length state would leak N and config.
- Decision: pad every note to `NOTE_PAD = 4096`.
- Alternatives: stock 512 (the state outgrows it from about N = 5, so the length would leak N).
- Consequence: the pad class marks our transactions (see D19).
## D11. `refund_to = None` and a private fee payer
- Context: a public refund account links the multisig.
- Decision: the private posture requires it; asserted in the byte scan.
- Consequence: fee remains public in amount.

## D12. Room: signed envelopes over a chat-module group; binding inside the room only
- Context: no LEZ-to-chat binding exists.
- Decision: the room carries the same signed envelopes as the CLI files. The spikes post `msig1:<kind>:` + base64 of the JSON envelope (the initial design's borsh encoding was dropped: one format for files and room). A `Bind` signed by the member's LEZ key over (V, chat address, room id), checked against the chain member list, never on chain. A new room per epoch because the chat module cannot remove members.
- Alternatives: on-chain binding (leaks the social graph); an external channel.
- Consequence: PLANNED (M3); spike only (`spikes/chat-module/run3`). Re-bind per launch, since the chat address changes; offline members miss messages, so the client must re-post pending envelopes on re-bind. File envelopes are the formal fallback.
## D13. Decoded signing from the signed bytes
- Context: U.6.
- Decision: the client decodes `borsh(Action)` from the envelope, renders it, recomputes the digest from the same bytes, signs that. Registry: a local JSON map program account -> name, built-ins native-token and the multisig. Unregistered target: `UNKNOWN program` and raw hex.
- Consequence: BUILT in the CLI. The draft's registry keyed by image id was not built: the registry maps accounts, and only for this program does the account follow from the image id. A second program's IDL and an explicit override before signing are PLANNED (M2); pins signed by k of N PLANNED (M3); GUI PLANNED (B4).
## D14. Time lock: single queue slot; spending limit: native only
- Context: R.2 cutoff with a delay; no token program on v0.4.
- Decision: one queued slot; while it is live no `Config` can land (it must be queued too), so a queued action never runs under another config. Only `Cancel` runs immediately under a time lock. `Spend` bypasses the time lock and has its own nonce.
- Alternatives: multi-queue voided by epoch (PLANNED, B1).
- Consequence: BUILT in image v2 / v2.1 (`Queue`, `ExecuteQueued`, `Spend`, `SetTimeLock`, `SetLimit`). Cost: one delayed action at a time, config included.
## D15. Selective disclosure by viewing key
- Decision: hand over the viewing secret key, never the GMS or `nsk`.
- Consequence: read-only, whole account, every member key exposed to the audience, no action history in the state (activity is only the sequence of decryptable past notes plus public effects), revocable only by a re-key that a GMS holder can stall. Test `viewing_key_alone_decrypts_state`; export command and history reconstruction PLANNED (M2).
## D16. Benchmark the whole private transaction with real proofs
- Decision: time `execute_and_prove` on a frozen image on the Linux GPU host; report cycles as a compute column only; unproven operations read "not benchmarked".
- Consequence: see `docs/BENCHMARKS.md`.

## D17. Private PDA at the deployed program account
- Context: the wallet helper binds a PDA to the built-in program address; ours is deployed at a key-derived account `P`.
- Decision: a near-copy of the wallet's shared-PDA creation taking `P` (vendored wallet crate; `vendor/PATCHES.md`).
- Consequence: `private_vault_is_bound_to_the_deployed_program_account`. Deploy key is public by derivation, so deploy before publishing the image id; a public testnet needs a private deploy key (PLANNED, M5).

## D18. Prototype deviations from the initial design
- Config ops built: `AddMember`, `RemoveMember{key}`, `SetThreshold`, `SetTimeLock`, `SetLimit(Option<Limit>)`. Removal by key (indices shift). `SetPerms` PLANNED (M2).
- State: `time_lock`, `limit`, `queued`, `spend` are in (`VERSION = 2`); `last` (approver bitmap) is not, and is in no milestone.
- `Execute` keeps its v1 encoding; a separate `ExecuteQueued` executes the queued slot.
- v1 states are rejected (`UnsupportedVersion`), not migrated; v1 vaults were dev vaults.
## D19. Note pad via a vendored constant, not a setting
- Decision: set the wallet constant to 4096 (`vendor/PATCHES.md` section 2).
- Consequence: every transaction from this wallet is distinguishable from stock-wallet (512) ones; the anonymity set is transactions padded to 4096, today this application. Stated in PRIVACY.md section 3.
## D20. Test-only `noop` guest for the veto test
- Decision: program X is `noop`, never deployed outside the harness, own image id.
- Consequence: `MSIG_ID` does not depend on it.

## D21. Image frozen from a Docker build
- Decision: frozen ids come only from the pinned reproducible builder (`scripts/image-ids.sh`); CI rebuilds and compares with the latest `evidence/IMAGE-ID-v*.md`. Current: v2.1 `e174a45f…34557` (`evidence/IMAGE-ID-v2.1.md`).
- Consequence: plain `cargo` builds embed host paths, so their ids differ. Every benchmark table is on v2.1.
## D22. Module surface and retry safety
- Context: the RFP requires a Logos module (U.1) and a headless run (U.3); the prototype ships the smallest module that exercises a full private flow.
- Decision: two calls, `create` and `execute`. `create` refuses to replace an existing group or `msig.json`; `execute` records the pending payment before submitting and reconciles it on the next call.
- Alternatives: full API now (M3).
- Consequence: see `docs/MODULE-API.md`; RED/GREEN logs `modules/headless/retry-red.log`, `retry-green.log`.

## D23. Chat spike scope
- Context: the room depends on the Logos chat module, whose behaviour on the target network was unknown.
- Decision: time-boxed spikes against the real chat module, no mock room; failures are committed as evidence.
- Alternatives: a mock room (would hide transport behaviour).
- Consequence: messages crossed on `logos.dev`; `logos.test` needs an RLN membership; catalog install hung, release lgx files installed by path. Run 3: 3 members, 2-of-3, real proposal and approvals through the group, forged approval dropped by the CLI, executed; an offline member did not receive a later proposal after restart (new address, `history_only`).
## D24. Scope exclusions
- Context: the RFP targets LEZ.
- Decision: single chain, LEZ only; the program and client are written for this design and do not reuse other multisig designs.
- Consequence: no cross-chain integration.
## D25. Benchmark setups
- Context: P.1 asks for client proving time on realistic hardware.
- Decision: real proofs on a desktop GPU (CUDA) and on a Mac, hardware named per row.
- Consequence: both on image v2.1. The Mac runs the CPU prover only: risc0 3.0.5 has no Metal proving path (`evidence/bench/mac-NOTE.md`). See `docs/BENCHMARKS.md`.

## D26. `MAX_MEMBERS = 30`
- Context: the worst case is a `Config` at M = N: N+2 BIP-340 checks plus N key decodes, which must fit the public-posture rule `gas_limit <= 10,000,000`.
- Decision: cap N at 30 (about 9.58M guest cycles; 9.88M at 31), enforced in `validate` at `Create` and every config op, so no configurable quorum is un-executable (`crates/msig-core/src/lib.rs:19-20`).
- Consequence: no configurable quorum exceeds the block cap in the public posture.

## D27. Image v2.1 fixes
- Decision: `time_lock <= MAX_TIME_LOCK` (30 days), since a larger lock would make every later `Queue` impossible; `SetLimit` resets the spend period counter but keeps the spend nonce; only the proposer queues (`QueuerNotProposer`), because approvers do not sign `t`.
- Consequence: image v2.1; tests `time_lock_is_capped_and_a_max_lock_still_queues_and_executes`, `set_limit_resets_the_period_counter_but_not_the_spend_nonce`, `only_the_proposer_queues_and_t_is_bound_by_the_queuer_signature`.

## D28. Status vocabulary
- Decision: documentation tags are BUILT (code plus a named test or evidence), PLANNED (milestone label from `proposal/MILESTONES.md`), NOT MET.
- Consequence: one vocabulary across the README, docs and `proposal/COVERAGE.md`.
