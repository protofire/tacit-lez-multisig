# Where the proving cycles of a private tx go (LEZ v0.4.0-rc1, image v2.1)

LEZ vendor 1806e87, risc0-zkvm 3.0.5, msig image
e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557 (asserted by msig-bench).
Executor only (`RISC0_DEV_MODE=1`), so these are exact cycle counts, not timings.

## Table

Same wallet path as `scripts/bench-mac.sh` (7-slot private padding, private fee payer, Undisclosed
program claim). "Padded" = sum of 2^po2 over segments, which is what the rv32im prover proves.
Segment limit po2 = 20 (risc0 default, LEZ does not set it).

| op | outer circuit user cycles | outer segments (padded) | inner msig user cycles | inner segments (padded) | inner share of padded | recursion proofs per tx | Mac M2 Max real prove (s) |
|---|---|---|---|---|---|---|---|
| fund (native only, no program) | 4,858,960 | 6 (5.77M) | 0 | 0 | 0 % | 11 (6 lift + 5 join) | 536 / 617 |
| Create M=5 (no sigs) | 4,858,392 | 6 (5.77M) | 231,942 | 1 (0.52M) | 8.3 % | 13 (+1 lift, +1 resolve) | 588 |
| Create M=10 | 4,909,186 | 6 (5.77M) | 381,824 | 1 (0.52M) | 8.3 % | 13 | 691 (excluded: overlapped other load) |
| Execute M=5 (7 sigs) | 4,941,823 | 6 (5.77M) | 1,923,743 | 2 (2.10M) | 26.7 % | 15 (+2 lift, +1 join, +1 resolve) | 738.6 (run 1; runs 2–3 excluded, see docs/BENCHMARKS.md) |
| Execute M=10 (12 sigs) | 4,968,836 | 6 (5.77M) | 3,276,593 | 4 (3.67M) | 38.9 % | 19 (+4 lift, +3 join, +1 resolve) | 900 (median of 3) |

Inner msig cycles match `msig-cycles` (M=5 1,930,308; M=10 3,286,859; the in-wallet Execute differs
by a few thousand cycles of input size). The signature-free "plumb" guest on the same Execute input
is 37,808 (M=5) / 48,711 (M=10) cycles: about 98 % of the inner program is Schnorr verification.
Real-prove seconds are from the clean CPU run (`evidence/bench/mac-cpu.log`, `prove_est_s`); fund/create were measured before the Execute runs and drift by up to
~15 % between the M=5 and M=10 blocks (536 vs 617 s for the same fund tx).

Rough fit: Mac CPU time is ~93-95 s per million padded cycles for every op (536/5.77, 588/6.29,
738.6/7.86, 900/9.44), so time follows padded cycles almost linearly; recursion steps are a small
fixed-ish add-on. Proof bytes are constant (267,166-267,514) because the receipt is succinct.

## How a private tx is proven (vendor/lez)

- Wallet: `send_tx` pads to `PADDED_PRIVATE_ACTIONS = 7` private actions with dummy inputs
  (lez/wallet/src/account_manager.rs:389, :671; lez/wallet/src/lib.rs:1543, :1603) and calls
  `lee::execute_and_prove` (lez/wallet/src/lib.rs:1766).
- `execute_and_prove` simulates the whole tx, then `prove`
  (lee/state_machine/src/privacy_preserving_transaction/circuit/mod.rs:370, :559).
- Each invoked program is proved first in `prove_session` with `default_prover().prove(env, elf)`
  (mod.rs:674-685), i.e. `ProverOpts::default()` = composite receipt
  (risc0-zkvm-3.0.5 src/host/client/prove/opts.rs:79-88). The native token program has no ELF;
  the circuit recomputes it (mod.rs:350-354), so fund/fee legs add no inner proof.
- The program receipt is added as an assumption (mod.rs:321) and the circuit calls `env::verify`
  on it (lee/privacy_preserving_circuit/src/private_backend.rs:77): a cheap in-circuit claim, the
  real cost is the recursion `resolve` at the end.
- The outer circuit (prebuilt `artifacts/lee/privacy_preserving_circuit/privacy_preserving_circuit.bin`,
  lee/state_machine/build.rs) is proved with `ProverOpts::succinct()` (mod.rs:652-657). risc0 then
  lifts every outer segment, joins them, compresses each composite assumption (lift + join its
  segments) and resolves it (risc0-zkvm-3.0.5 src/host/server/prove/mod.rs:353-388).
- Executor segment size: `ExecutorEnv::segment_limit_po2` unset, so `DEFAULT_SEGMENT_LIMIT_PO2 = 20`
  (risc0-zkvm src/host/server/exec/executor.rs:186-189; risc0-circuit-rv32im-4.0.5
  src/execute/mod.rs:39).

## Knobs

- No `segment_limit_po2` or `ProverOpts` knob in the wallet path: both env builders are fresh
  `ExecutorEnv::builder()` (mod.rs:571, :678) and the opts are hard-coded (mod.rs:655, default at :685).
  risc0 3.0.5 reads no env var for po2 either.
- Env knobs that exist: `RISC0_PROVER` (local / ipc / actor / bonsai), `RISC0_DEV_MODE`,
  `RISC0_DEFAULT_PROVER_NUM_GPUS`, and the build features `metal` / `cuda`.
- Ours: the inner cost is almost all signature checks (sigs = M+2 per Execute); fewer checked
  signatures or a cheaper scheme is the only lever on our side.
- LEZ-side (not ours to change): the 7-slot dummy padding and the circuit itself; the cost of the
  padding was not measured here (LEZ `tools/cycle_bench --ppe` sweeps dummy counts).

## Method / commands

1. `crates/msig-host/src/bin/msig-bench.rs` now calls `env_logger::init()` (silent unless
   `RUST_LOG` is set); `env_logger` is an optional dep under the `e2e` feature. No other change.
2. Build (macOS 27 fixes as in `scripts/bench-mac.sh`):
   `RISC0_USE_DOCKER=1 RECURSION_SRC_PATH=<complete recursion_zkr.zip, only if the download is truncated> cargo build --release --config 'patch.crates-io.risc0-sys.path="target/risc0-sys-metal-fix"' -p msig-host --no-default-features --features e2e,prove --bins`
   then restore Cargo.lock.
3. Dev-mode replica of bench-mac (sequencer `sequencer_service` from LEZ 1806e87 on 127.0.0.1:3076,
   `RISC0_DEV_MODE=1 BENCH_MODE=dev RISC0_INFO=1 RUST_LOG=risc0_zkvm=info`):
   `msig-bench init; create 5 7 c5; fund c5 100000; exec c5; create 10 12 c10; fund c10 100000; exec c10`.
   risc0 logs `Session::log` (segments, total/user/paging/reserved cycles) for every executor run.
   Per op the inner msig session appears 4 times (dry-run, fee simulation, `simulate_whole`,
   `prove_session`); only the last is proved. The 6-segment session is the outer circuit.
4. `target/release/msig-cycles 5 10` for the standalone guest numbers.

## Conclusion

1. The LEZ v0.4 privacy circuit dominates: ~4.9M user / 5.77M padded cycles (6 segments) in every
   private tx, 61-100 % of what gets proved; a fund tx with no program at all already takes 536 s on the Mac.
2. Our program is 8 % (Create) to 27 % (Execute M=5) to 39 % (M=10) of proved cycles, and ~98 % of
   that is Schnorr signature verification (M+2 sigs).
3. Prove time tracks padded cycles at ~95 s per million on the M2 Max CPU (all 12 cores), plus 11-19
   recursion proofs per tx (lift/join/resolve), which grow with segment count.
4. The circuit itself is not heavier than LEZ 0.3; the v0.4 fee leg adds about 1.8M user cycles
   (`evidence/fee-leg-cost.md`).
5. No prover knob is exposed in the wallet path (po2 and receipt kind are hard-coded); the levers
   are hardware (metal/cuda, GPU ~10x faster) and our own signature count.
