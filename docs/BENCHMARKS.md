# Benchmarks (RFP-005 P.1, P.2)

All figures are on image v2.1:
- **Laptop CPU:** Apple M2 Max, clean run after a reboot. Raw log `evidence/bench/mac-cpu.log`, note `mac-NOTE.md`.
  Script `scripts/bench-mac.sh`.
- **Desktop GPU:** RTX 3080. Raw logs `evidence/bench/real-run.log`, `real-env.log`, `cycles.log`, `cycles-env.log`.
  Script `scripts/bench-gpu.sh`.
- **Whole-tx cycle breakdown** (LEZ privacy circuit vs our program): `evidence/tx-cycles.md`.

Common to both machines: real proofs (`RISC0_DEV_MODE` unset), submitted to a local standalone LEZ v0.4.0-rc1 sequencer that
verified the receipts; image v2.1 `e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557`, asserted at runtime; same
flow, checks and timing method on both (see "Method" below: proving call = `send_tx` minus dry run; e2e =
sync + `send_tx` + the wallet's fixed 30 s inclusion poll). All proven txs were included, none reverted.

## Image v2.1, laptop CPU (Apple M2 Max)

- **Machine:** Apple M2 Max (Mac14,5), 8P+4E cores, 32 GB, macOS 27.2.1; rustc 1.98.1; risc0-zkvm 3.0.5.
  After a reboot: swap 0, Docker stopped, 79 % memory free at start.
- **CPU prover only.** risc0-zkvm 3.0.5, pinned by LEZ v0.4.0-rc1, has no Apple-GPU path for the main prover: in
  risc0-circuit-rv32im 4.0.5 the Apple GPU branch of `segment_prover()` is commented out, and the `metal` feature is deprecated
  (`mac-NOTE.md`).
- **Core use:** system-wide CPU 1062–1094 % average per proving step, about 10.7 of 12 cores.
- **Interference:** a video call ran during Execute M=5 runs 2–3 and Create M=10; those runs are excluded.

| Operation | M (N) | proving call, s | e2e client, s | peak RSS | tx bytes | proof bytes |
|---|---|---|---|---|---|---|
| private Execute | 5 (7) | 738.6 (run 1) | 768.7 | ≤ 12.3 GB | 311,670 | 267,398 |
| private Execute | 10 (12) | 899.9 (896.9-926.7) | 930.1 (927.0-956.9) | ≤ 12.3 GB | 311,670 | 267,398 |
| private Execute | 30 (30) | not run | not run | — | — | — |
| private Create | 5 (7) | 588.1 | 618.3 | ≤ 12.3 GB | 311,206 | 267,166 |
| private fund (public payer -> private vault) | — | 535.9 | 566.0 | ≤ 12.3 GB | 312,046 | 267,514 |

Execute M=10: median of 3 measured runs after a warm-up (range in brackets). Execute M=5: run 1 (the warm-up gave 738.4 s).
Peak RSS is the largest `max_rss_mib` of any operation (12,320 MiB, fund). Only the submitting member proves, so only the
submitter needs this hardware; approvers only sign. An earlier session on the same machine before the reboot (11 days uptime,
swap full) gave about 766 s / 917 s for Execute M=5 / 10 (`mac-metal.log`, also CPU prover).

## Image v2.1, desktop GPU (RTX 3080)

WSL2 Ubuntu, Linux 6.18.54.1, x86_64, 12 threads, 31 GiB RAM; NVIDIA RTX 3080 12,288 MiB, driver 617.14, CUDA 12.8; rustc 1.98.1;
risc0-zkvm 3.0.5 with `cuda` (in-process GPU prover). Environment: `evidence/bench/real-env.log`.

| Operation | M (N) | proving call, s | e2e client, s | peak GPU memory (total) | peak RAM | tx bytes | proof bytes |
|---|---|---|---|---|---|---|---|
| private Execute | 5 (7) | 70.0 (69.5-70.2) | 100.1 (99.6-100.3) | 11,066 MiB | 744 MiB | 311,670 | 267,398 |
| private Execute | 10 (12) | 80.7 (80.2-83.3) | 110.8 (110.4-113.5) | 11,108 MiB | 745 MiB | 311,670 | 267,398 |
| private Execute | 30 (30), 1 warm-up + 1 run | 145.0 (warm-up 131.9) | 175.4 | 11,066 MiB | 751 MiB | 311,670 | 267,398 |
| private Create | 5 (7) / 10 (12) / 30 (30) | 53.7 / 54.6 / 63.3 | 83.8 / 84.6 / 93.3 | 11,066 MiB | 745 MiB | 311,206 | 267,166 |
| private fund | — | 52.6 (52.2-52.9) | 82.6 (82.2-82.9) | 11,066 MiB | 696 MiB | 312,046 | 267,514 |

Execute M=5/10: median of 3 measured runs after a warm-up. Create: one run per M. Fund: median of the three runs (one per vault).
GPU memory includes about 1.26 GiB in use before each run.

## Where the proving time goes (`evidence/tx-cycles.md`)

Every private tx proves the LEZ v0.4 privacy circuit at about 5.77M padded cycles (6 segments), independent of our program; a fund
tx, which runs no multisig program, already takes 536 s on the M2 Max CPU. Our program's share of proved cycles: Create 8 %,
Execute M=5 27 %, M=10 39 %, about 98 % of it signature checks. On the M2 Max CPU, proving costs about 93-97 s per million padded
cycles; 11-19 recursion proofs per tx. The receipt is succinct, so the proof is about 267 KB in every operation. The wallet path
exposes no segment-size or prover-options setting (defaults are hard-coded). Per extra signature (M=5 -> 10): about 2 s on the
GPU, about 30 s on the laptop CPU.

## P.2 approval-verification cost vs M (image v2.1, executor, `cycles.log`)
`msig-cycles`: the `msig` guest executed on a `ReceiveInput` for `Execute` with M approvals (members 0..M-1, N = min(M+2, 30)),
user cycles = `SessionInfo::cycles()` (what LEZ meters for public programs); "plumb" = `crates/msig-plumb`, a test-only guest
(own image id `057396b3…f410`, never deployed) doing the same decode/digest/state/response work without the signature checks.

| M | signatures | msig user cycles | segments | plumbing cycles | crypto = msig - plumb | per signature |
|---|---|---|---|---|---|---|
| 1 | 3 | 840,849 | 1 | 29,193 | 811,656 | 271k |
| 2 | 4 | 1,112,578 | 2 | 31,408 | 1,081,170 | 270k |
| 5 | 7 | 1,930,308 | 2 | 37,808 | 1,892,500 | 270k |
| 10 | 12 | 3,286,859 | 4 | 48,711 | 3,238,148 | 270k |
| 16 | 18 | 4,920,124 | 5 | 59,911 | 4,860,213 | 270k |
| 24 | 26 | 7,092,998 | 8 | 76,250 | 7,016,748 | 270k |
| 30 | 32 | 8,711,706 | 9 | 87,504 | 8,624,202 | 270k |

Cost is linear: about 270k cycles per BIP-340 check plus ~0.03-0.09 M cycles of plumbing; signatures are over 98 % of the guest.
The rule: `cycles(M) ~= 0.03M + 0.27M x (M+2)`.

Worst case (`worst_case_quorum_fits_the_block_gas_cap` in `crates/msig-host/tests/private.rs`, executor, no proving): a `Config`
at M = N, the most expensive action, uses 9,581,318 cycles at 30 (a `Call` 8,711,706), 9,883,700 at 31 and 10,183,565 at 32, over
the 10M cap. Hence `MAX_MEMBERS = 30`, enforced at `Create` and every config op (`member_count_is_capped_at_max_members`), with
about 4 % headroom.


## Method
- **Image check:** guests built with `RISC0_USE_DOCKER=1`; every bench binary asserts at runtime that the msig image is
  `e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557` (v2.1) and refuses otherwise.
- **Sequencer:** LEZ 1806e87 (v0.4.0-rc1) `sequencer_service`, standalone, port 3075, **RISC0_DEV_MODE unset** so the sequencer
  verifies the receipts.
- **What is submitted:** `msig-bench` drives the LEZ wallet (patched, vendor/PATCHES.md) as the e2e does: private fee payer
  (shielded 1e11), fee inbox, the Undisclosed program claim, notes padded to `NOTE_PAD`. Each tx goes to the sequencer; the
  bench waits for inclusion, asserts not reverted, `refund_to None`, note length == `NOTE_PAD`, only Undisclosed claims, no `P`
  or image id in the tx bytes, and for Execute the vault balance -1, recipient +1, nonce +1 (`CHECK` lines).
- **Timed (one operation = one fresh process, so RSS/GPU peaks are per operation):** `sync` (wallet sync, excluded);
  `dry_run` (the same `send_tx` with the wallet's dry-run: everything before proving, 0.03-0.16 s); `send_tx` = simulate +
  fee pricing + **proving** + sign + submit; `incl` = wait for inclusion. **Proving call = `send_tx` - `dry_run`**
  (the wallet runs `lee::execute_and_prove` inside `send_tx`; it cannot be timed alone without patching vendor/).
  **End-to-end client time = sync + send_tx + inclusion.** `incl` is 30.0 s in every row: that is the wallet's
  `poll_transaction` interval, not block time; read e2e minus 30 s as the client time to submission.
- **Size:** `tx bytes` = `len(borsh(PrivacyPreservingTransaction))` of the included tx; `proof bytes` = its borsh proof minus the
  4-byte length. **Peak RAM:** `/usr/bin/time` of the process. **Peak GPU:** `nvidia-smi` memory.used sampled every 0.5 s
  during the process, including the baseline in use before the run.
- **Runs:** each row is a separate process; 1 warm-up + 3 measured, median (min-max), unless the row says otherwise.

## Dev-mode control (NOT a benchmark)
`RISC0_DEV_MODE=1` on a dev-mode sequencer (`dev-run.log`, an earlier image build, same flow), Execute M=5, N=7: proving call
0.32 s, process wall 30.8 s (inclusion poll), tx **88,657 B** with a **44,385 B** receipt (vs 311,670 B / 267,398 B real), peak
RAM 58 MiB, GPU unused. The real rows are therefore real succinct proofs. Do not cite dev-mode figures.

## Gas rule and largest M per posture
- **Block gas:** `DeclaredGasBudget::fits` admits a tx while `exec + gas_limit <= MAX_GAS_EXEC` (`vendor/lez/lez/sequencer/core/src/lib.rs:143-144`);
  `MAX_GAS_EXEC = 10,000,000` (`lez/programs/fee/core/src/market.rs:15`; target 5M). A public tx's charge is its metered cycles.
- **Public posture (not built yet; extrapolated from the executor figures above, not measured on a sequencer):** the whole msig guest
  costs 8.71 M cycles for an Execute at M=30 (32 signatures, image v2.1), under 10 M, leaving ~1.3 M for the rest of the public tx (the native-token call
  and its fee handling were not measured). On image v2.1 the worst case (a `Config` at M = N) sets the
  member cap `MAX_MEMBERS = 30` (9.58 M cycles, above). A single public action near the cap takes almost the whole block: two
  Executes per block only up to M ~ 17 (2 x 4.9 M at M=16). Largest viable M, public: **30 (cap by `MAX_MEMBERS`)**,
  with the stated headroom risk.
- **Private posture:** not metered by cycles. The sequencer charges a private tx a flat `PRIVATE_VERIFY_GAS = 409,764` plus
  `PRIVATE_GAS_STOR = 224,063` (`market.rs:49-56`, `assess.rs:84-88`); the approval cost is client proving time. Image v2.1 on the
  RTX 3080 (12 GiB, 12 threads): M=5 70 s, M=10 81 s, M=30 145 s; RAM ~0.75 GiB and GPU memory ~11 GiB total at every M.
  On the Apple M2 Max CPU: M=5 739 s, M=10 900 s, ≤ 12.3 GB RSS. Largest viable M, private: **30** (all of
  N <= `MAX_MEMBERS`), bounded by proving time not by block limits. M >= 5 is viable in both postures.
- Note: real private txs are 311,670 B and carry a 267,398 B proof, above the 223,551 B `PROOF_BYTES` constant the fee model assumes;
  the sequencer included them regardless (v0.4.0-rc1, the constant is marked provisional in `market.rs`). About 3 fit a 1 MiB block.

## Not done / limits
- Laptop: CPU only (no Apple-GPU prover path in risc0 3.0.5); Execute at M=30 not run; Execute M=5 is a single run (runs 2-3
  overlapped a video call).
- Desktop GPU: Execute M=30 and Create are single runs.
- No real-proof run yet of a config change, `Queue`/`ExecuteQueued` or `Spend`.
- Proving call is `send_tx` minus a dry run, not an isolated timer; sequencer-side verification time is not reported.
- Public-posture figures are executor cycles extrapolated to the gas rule; no public multisig exists yet.
- GPU memory is a 0.5 s `nvidia-smi` sample including other processes' baseline.
- Plumbing guest handles `Call` actions only.
