# RFP-005 milestones: deliverables and acceptance criteria

Companion to `COVERAGE.md`. Requirement IDs are RFP-005's: F = Functionality, U = Usability, R = Reliability, P = Performance, S = Supportability, Pr = Privacy.

A criterion is met when a Logos reviewer can reproduce it from the repository at the milestone tag, using the command or document the criterion names.

## Structure

- Milestones **M1–M6** deliver the core.
- Four **base lines, B1–B4**: time lock (F.7), spending limit (F.8), public posture on a live sequencer (F.10, public half), and the QML mini-app with final designs (U.2, S.11, and the front-end parts of U.6 and S.8). Each can be removed independently; its parts then also leave the M4 documentation, the M5 benchmarks and the audit scope. No milestone needs a base line to pass. B4 depends on the M3 room.
- **Only the soft requirements are optional:** O1, on request.
- **M1 is the RFP's suggested first phase:** the approval-verification benchmark, the design constraints that follow, and three written go/no-go gates. A gate can lead to a re-scope agreed with Logos (the branches are below).
- **Calendar:** about 20 weeks in all. B lines run in parallel with the milestones.

| # | Item | Weeks | Requirements | Status today |
|---|---|---|---|---|
| M1 | Benchmark, design constraints, three go/no-go gates | 1–2 | P.1, P.2 (phase 1), F.1 | PARTIAL: laptop CPU and desktop GPU figures on image v2.1 measured (GPU incl. M = 30); `MAX_MEMBERS = 30` derived and tested; laptop M = 30, config change, timer and gates open |
| M2 | Full program and per-requirement tests | 3–7 | F.1–F.6, F.10 (private posture, disclosure), R.1, R.2, U.3 (CLI), U.6 (CLI), U.7, S.7, S.10 | PARTIAL: quorum, roles, five config ops, cancel, replay and epoch tests built (77 tests pass); `SetPerms`, second-program call, CLI `Call`/`cancel`, viewing-key export open |
| M3 | Room on the chat module; every module and CLI operation headless | 6–10 | F.2 (room transport), F.4 (pins), F.9, U.1, U.3 (headless), S.2, S.9, Pr.2, Pr.3 | PARTIAL: 2-of-3 room spike with real approvals executed on chain; module `create`/`execute` headless; room protocol and full API open |
| M4 | Docs, own catalog, signed package, IDL | 11–12 | S.8, S.12, S.14, U.4, U.5, U.6 (signed package) | PARTIAL: README, CLI, privacy docs, decision log and SPEL-format IDL with drift check exist; catalog and signing open |
| M5 | Testnet 0.4 deployment and tests, final benchmark, CI; code freeze | 13–14 | S.3, S.4, S.5, S.6, P.1, P.2 (final), S.13 (freeze) | PARTIAL: CI workflow written; testnet deployment open |
| M6 | Audit remediation and re-review | 19–20 | S.13 | PLANNED |
| B1 | Base line, separable: time lock, finishing (multi-action queue) | 3–5 | F.7, R.2 (time-lock cutoff) | PARTIAL: single-slot `Queue`/`ExecuteQueued`, `SetTimeLock`, 30-day cap, tests and CLI built |
| B2 | Base line, separable: spending limit, finishing | 3–4 | F.8 | PARTIAL: `Spend` under a per-period native limit, `SetLimit`, tests and CLI built |
| B3 | Base line, separable: public posture on a live sequencer | 6–8 | F.10 (public posture), P.1, P.2 | PARTIAL: public PDA tested at handler level; not run on a sequencer |
| B4 | Base line, separable: QML mini-app and final designs | 9–14 | U.2, U.6 (GUI), S.8 (front-end), S.11, S.12 (mini-app) | PARTIAL: static HTML design mock; QML app open |
| O1 | *Optional:* soft requirements (weighted approvals, mixed postures) | on request | Soft F.1, Soft F.3 | PLANNED |
| — | External audit (separate line, not part of the development plan) | 15–18 audit, 19–20 fixes and re-review | S.13 | PLANNED |

Status today: BUILT = done and evidenced in the repository; PARTIAL = part built (named), the rest is the milestone; PLANNED = not built.

## Already built

The prototype in this repository is supporting evidence, not a deliverable. Per-row status is in `COVERAGE.md`. In short, at frozen image v2.1 (`evidence/IMAGE-ID-v2.1.md`) it contains:
- the in-guest M-of-N quorum with role bits, epoch and nonce, `Cancel`, and the config ops add member, remove member, set threshold, set time lock and set limit;
- `MAX_MEMBERS = 30`, derived from the worst-case cycles and enforced by a test;
- a single-slot time lock and a per-period spending limit for native balance;
- a private multisig on a local LEZ v0.4.0-rc1 sequencer at the deployed program address;
- the quorum-bypass and liveness-veto tests;
- the transaction byte scan and fixed-length notes;
- viewing-key decryption;
- the `msig` operator CLI with file envelopes and decoded signing;
- the Logos module and headless `create`/`execute`;
- real-proof benchmarks on image v2.1 on a laptop CPU (M = 5 and 10) and on a desktop GPU (M = 5, 10 and 30);
- a 2-of-3 room spike on the chat module with real approvals executed on chain;
- the reproducible image id;
- a SPEL-format IDL, a CI workflow, privacy and adversary docs, a decision log and a UI mock.

The milestones below cover what remains.

## M1 — Benchmark, design constraints, three go/no-go gates (weeks 1–2)

| Criterion | IDs | Status today |
|---|---|---|
| Real-proof figures (`RISC0_DEV_MODE` unset, receipts verified by the sequencer) for private `Execute` at M = 5, 10 and 30, `Create`, fund and one config change, on image v2.1: on a laptop (Apple M2 Max, CPU prover, idle machine) and on a desktop GPU. Same method and columns as `docs/BENCHMARKS.md`. Reproducible with `scripts/bench-mac.sh` and `scripts/bench-gpu.sh`. Failed attempts are reported with their logs. | P.1, P.2 | PARTIAL: laptop (clean run after reboot) `Execute` M = 5/10, `Create`, fund measured; GPU `Execute` M = 5/10/30, `Create`, fund measured; laptop M = 30 and a config change open |
| An isolated proving timer, replacing the `send_tx` minus dry-run difference. Sequencer-side verification time reported. A bytes-per-block row against the 1 MiB maximum block size. | P.1 | PLANNED |
| Design-constraints report: the largest viable M per posture and its binding limit; `MAX_MEMBERS` derived from it and enforced by a test, so no quorum that cannot execute can be configured; the headroom left in the 10M block for the public-posture call; the proving time and memory operators should expect on a laptop. | P.2, F.1 | PARTIAL: `MAX_MEMBERS = 30` from `worst_case_quorum_fits_the_block_gas_cap`, enforced by `member_count_is_capped_at_max_members` |
| Questions to Logos, each with a reproducer, answers recorded in `docs/DECISIONS.md`: whether a public testnet 0.4 exists; chat-module transport metadata; the provisional `PROOF_BYTES` fee constant, which real transactions exceed; whether SPEL supports v0.4 actor programs. | S.4, Pr.2, P.1, U.4 | PLANNED |
| Both wallet patches submitted as LEZ pull requests; issues filed for the nullifier liveness veto, RLN membership on `logos.test`, and `PROOF_BYTES`. | — | PLANNED |
| The three gates below, each closed with a written decision signed off by Logos. | — | PLANNED |

**Gate (a): laptop viability.** Pass: a private `Execute` at M = 5 proves on a laptop within a bound agreed with Logos at kickoff. We propose 15 minutes and 16 GB of RAM on an Apple M2 Max-class CPU. Today: 738.6 s (about 12.3 minutes) and at most 12.3 GB peak RSS, measured after a reboot.
- Pass: no change.
- Miss: the finding is the deliverable and the rest goes to a scope discussion with Logos (a GPU or remote prover for the submitting member, or a lower operator limit), before M2 starts.

**Gate (b): the removed-member liveness veto.** Logos answers whether LEZ will add an owner-gated nullifier advance or a read-only witness, with the reproducer `gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced`.
- Branch 1, LEZ fix planned before M5: we adopt it in the program and client in M2/M5.
- Branch 2, no LEZ fix in this RFP's window: the private posture ships with the freeze risk stated in the CLI, the docs and the mini-app, plus the operator procedure (removal followed by re-key, GMS held only by `EXECUTE` members, voters get the viewing key; evaluated in M1). B3 (public posture) becomes non-removable, as the posture for treasuries that cannot accept the risk.
- Branch 3, Logos asks us to implement the LEZ change: outside this plan, scoped separately after the gate.

**Gate (c): the chat module on the 0.4 network.** Pass: a chat-module release usable on the network Logos pairs with testnet 0.4, including any RLN membership it needs.
- Pass: M3 runs on that network.
- Miss: M3 is delivered on `logos.dev` with envelope files as the fallback transport. The 0.4-network criterion of M3 moves until it can run.

## M2 — Full program and per-requirement tests (weeks 3–7)

| Criterion | IDs | Status today |
|---|---|---|
| `SetPerms` config op through the M-of-N flow. `validate` rejects `perms == 0` and unknown permission bits. | F.5, F.6 | PARTIAL: perms validation built (`zero_and_unknown_perm_bits_are_rejected`); `SetPerms` open |
| `Execute` of a call to a second, non-native test program on the sequencer, with and without authority. Authority on a foreign account is rejected. The CLI proposes arbitrary `Call` actions and a `cancel`. | F.3 | PARTIAL: `Call` and `Cancel` in the program; on chain only native transfers; CLI `Call`/`cancel` open |
| The registry holds the second program's IDL, and the CLI decodes its call from the signed bytes before signing. Unregistered targets need an explicit override before signing. Decoding refuses tampered bytes. | F.4, U.6 | PARTIAL: decoder, `UNKNOWN program`, `TAMPERED` built |
| CLI completed: sealed distribution of the vault file; encrypted member key files. | F.2, U.3 | PLANNED |
| An on-chain test that an expired proposal is rejected. | F.2 | PLANNED (client-side check only) |
| Every program and CLI error maps to a message that says what to do next; a test covers every code. | U.7 | PARTIAL: typed errors and CLI refusals built |
| Viewing-key export command. An audience holding only that key verifies config and balance, and does so again after later transactions (ongoing assurance). | F.10 | PARTIAL: `viewing_key_alone_decrypts_state` |
| Re-key flow documented and tested: a new multisig under a fresh group secret, assets moved by normal M-of-N actions. | F.10, Pr.1 | PLANNED |
| A test per F, U and R requirement, mapped in `COVERAGE.md`, including the three tests S.7 names. F.9, U.1, U.2 and U.3 (headless) are covered in M3 and B4. | S.7 | PARTIAL: the three S.7 tests exist |
| CLI documentation of the full operator journey. | S.10 | PARTIAL: `docs/CLI.md` |

## M3 — Room on the chat module; every module and CLI operation headless (weeks 6–10)

| Criterion | IDs | Status today |
|---|---|---|
| Room messages (`Bind`, `Invite`, `Propose`, `Approve`, `Reject`, hints) carry the same envelope format the CLI uses for files. Tests: `Bind` is verified against chain state; a `Bind` for another `V` is rejected; forged, replayed and reordered envelopes have no effect on chain. | F.9, F.2 | PARTIAL: envelopes crossed the room and a forged approval was dropped (`spikes/chat-module/run3`) |
| A room provisioned from a multisig's member set, with at least 3 members: invite, accept after recomputing `V`, a proposal and M signed approvals carried through the room, execution on the sequencer. Transcript committed. | F.9, S.2 | PARTIAL: run 3 did this with 3 members, without `Bind` or sealed invites, vault file handed over on disk |
| Restart and re-bind: a member's daemon restarts, gets a new chat address, re-binds, and takes part in the next proposal. | F.9 | PLANNED (run 3 showed the new address and `history_only` room) |
| Offline member catch-up: a member offline while a proposal and approvals are posted receives them later and can approve. If the chat module cannot deliver them, the client recovers them from envelope files, and the transcript shows which. | F.2, F.9 | PLANNED (run 3: the offline member received nothing in 180 s) |
| The room runs on the chat network paired with testnet 0.4 (gate (c)). | S.2, S.3 | PLANNED (runs so far on `logos.dev`) |
| Reconciliation on every config change: a new room per config epoch, re-bind, and a removed member shown as removed. What a removed member can still read is documented and tested. | F.9 | PLANNED |
| Adversary table (withhold, reorder, replay, inject, split view) backed by tests where the program or client decides. | F.9, Pr.3 | PARTIAL: `docs/ADVERSARY.md`; program-side tests exist |
| Transport leak check: what a non-member observer of the delivery layer sees for room traffic, measured. Result in `docs/PRIVACY.md`. | Pr.2, Pr.3 | PLANNED |
| Full module API (create, propose, approve, reject, execute, manage members and policies). `msig` becomes a front-end over the module through Logos core; all six operations and the config changes run headless under `logosctl` in one transcript. API docs cover the integration journey. | U.1, U.3, S.9 | PARTIAL: `create` and `execute` headless |
| Registry pins signed by k of N members. | F.4 | PLANNED |

## M4 — Docs, own catalog, signed package, IDL (weeks 11–12)

| Criterion | IDs | Status today |
|---|---|---|
| Own module catalog built from the Logos module catalog template; the module installs from it with `logosctl`. Linux and macOS builds. | S.12 | PLANNED |
| The module package (`.lgx`) is signed, and the catalog and client check the signature before loading. | U.6, S.12 | PLANNED (unsigned today) |
| SPEL IDL for every instruction, drift-checked. Generator and client interop shown; if SPEL does not support v0.4 actor programs, the limit is documented and signed off by Logos. | U.4 | PARTIAL: `idl/msig.idl.json`, drift check, SPEL struct load |
| README end to end: deployment steps, create, propose, approve and execute via the CLI, and room provisioning. | S.8 | PARTIAL: prototype path |
| Operator documentation of public and private data per action, per posture and per disclosure mechanism. | U.5 | PARTIAL: `docs/PRIVACY.md`, ARCH §3.2 |
| Decision log current. | S.14 | BUILT, kept current |

## M5 — Testnet 0.4 deployment and tests, final benchmark, CI; code freeze (weeks 13–14)

| Criterion | IDs | Status today |
|---|---|---|
| Program deployed on public LEZ testnet 0.4. Create, propose, approve and execute shown there with real proofs; program addresses in the README. Conditional on Logos operating a public 0.4 testnet. If none exists, this criterion moves until one does, and the matching part of M5 is accepted then. | S.3, S.4, S.8 | PLANNED |
| CI green on GitHub on the default branch: tests, IDL drift, reproducible image id, and the private e2e against a standalone sequencer. | S.5, S.6 | PARTIAL: `.github/workflows/ci.yml` written, not yet run |
| Final real-proof benchmarks on the code under audit, for every on-chain operation (including those of B1–B3 kept in scope), with the same method as M1. | P.1, P.2 | PLANNED |
| Code frozen and tagged at the end of this milestone. Audit scope document (the list below, under M6) handed to the auditor. | S.13 | PLANNED |

## M6 — Audit remediation and re-review (weeks 19–20)

| Criterion | IDs | Status today |
|---|---|---|
| Fixes for all findings, re-review by the auditor, and the audit report published in the repository. No mainnet recommendation and no production deployment before this. | S.13 | PLANNED |

The audit itself (weeks 15–18, a tier-1 firm) is a separate line. Its scope: program and approval verification, vault initialisation, config change and epoch cutoff, time lock (B1), spending limit (B2), the client code that builds and checks the signed bytes, and the two wallet patches. If Logos removes B1 or B2, that logic leaves the scope.

## Base lines, separable

Each line has its own tests, documentation rows and audit-scope addition.

### B1 — Time lock, finishing: multi-action queue (weeks 3–5)

| Criterion | IDs | Status today |
|---|---|---|
| `Queue` and `ExecuteQueued` under a per-multisig time lock, with the queue time `t` signed by the proposer. An early execute is rejected; time lock 0 executes at once. `SetTimeLock` goes through the M-of-N flow, capped at 30 days. | F.7, F.5 | BUILT, single slot: `time_lock_blocks_early_execute_then_executes_at_eta`, `queue_then_execute_after_eta`, `time_lock_is_capped_and_a_max_lock_still_queues_and_executes`, `only_the_proposer_queues_and_t_is_bound_by_the_queuer_signature` |
| Several queued actions, each recording the config epoch it was approved under. A config change that executes voids every other queued entry. Tests cover a member removed during a delay and the cutoff ARCH §2.6 defines. CLI commands and docs for the queue. | R.2, U.3 | PARTIAL: single slot voided by an epoch bump (`queued_action_is_voided_by_cancel_and_by_an_epoch_bump`); multi-queue open |

### B2 — Spending limit, finishing (weeks 3–4)

| Criterion | IDs | Status today |
|---|---|---|
| `Spend` by a sub-quorum of `SPEND` members, up to a limit per period, for native balance. Tests: within the limit, over the limit, a past period cannot be backfilled, zero threshold or period rejected. `SetLimit` goes through the M-of-N flow. | F.8, F.5 | BUILT: `spend_within_limit_moves_native_and_keeps_the_action_nonce`, `spend_over_limit_is_rejected_and_resets_on_a_new_period`, `spend_period_cannot_be_backfilled`, `bad_limits_are_rejected`, `set_limit_resets_the_period_counter_but_not_the_spend_nonce` |
| `Spend` and `SetLimit` with real proofs on the sequencer; `Spend` through the module API; CLI docs. Other assets once a token program exists on LEZ v0.4. | F.8, P.1 | PARTIAL: CLI `spend` and `config set-limit` built (dev mode) |

### B3 — Public posture on a live sequencer (weeks 6–8)

| Criterion | IDs | Status today |
|---|---|---|
| Create, execute and config change of a public-posture multisig on the sequencer, with the same program. Gas per operation measured against the 10M block cap at M = 5, 10 and 30 (the current public figures are from executor cycles). | F.10, P.1, P.2 | PARTIAL: public PDA derivation tested at handler level |

### B4 — QML mini-app and final designs (weeks 9–14)

| Criterion | IDs | Status today |
|---|---|---|
| Final designs (Figma or equivalent) of the proposal list beside the room, decoded signing, and the proving state. | S.11 | PARTIAL: static HTML mock `docs/ui/mini-app.html` |
| QML mini-app loadable in Basecamp from the git repo, with local build instructions and downloadable assets. The proposal list and the room are side by side. Before signing, the action is decoded from the exact signed bytes through the registry. Proving runs in the background with visible progress. Uses the M3 room. | U.2, U.6 | PLANNED |
| The mini-app published in the own catalog (M4); README front-end steps. | S.12, S.8 | PLANNED |

## Optional

### O1 — Soft requirements (on request)

Weighted approvals (Soft F.1) and mixed postures (Soft F.3), scoped only if Logos asks. Proof of holding (Soft F.2) is not offered.
