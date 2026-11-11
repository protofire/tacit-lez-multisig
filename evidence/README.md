# Evidence index

All files are on guest image v2.1 (`e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557`) unless the row says otherwise. Transcript headers record the image id and the LEZ version (v0.4.0-rc1, `1806e87`) they ran against. Local paths are redacted (`<repo>`, `$LEZ_SEQUENCER_BIN`).

| File | Shows | Produced by | Proofs |
|---|---|---|---|
| IMAGE-ID-v2.1.md | frozen guest image id, builder pin, guest input hashes, how to reproduce | `scripts/image-ids.sh` | n/a |
| cargo-test-workspace.txt | `cargo test --workspace` output | `cargo test --workspace` | dev mode |
| e2e-private-transcript.txt | private-posture end to end against a local sequencer | `scripts/e2e-private.sh` | dev mode |
| cli-walkthrough.txt | 3-operator CLI walkthrough incl. tamper, limit and time-lock cases | `scripts/cli-walkthrough.sh` | dev mode |
| module-headless-transcript.txt | Logos module `create` and `execute` under `logosctl` | `modules/headless/headless.sh` | dev mode |
| leak-surface.md | per-field leak scan of a private `Execute` (N = 3, N = 30) | test `private_tx_byte_scan` | dev mode |
| tx-cycles.md | where the proving cycles of a private tx go (privacy circuit vs this program) | `msig-bench` in dev mode with risc0 session logs | executor only |
| fee-leg-cost.md | cost of the v0.4 fee leg in the LEZ privacy circuit, same transfer on v0.3.0, dev 9f004c6b and v0.4.0-rc1 | small test added to upstream LEZ circuit tests | executor only |
| pad-fee-cost.md | cycle and size cost of the note pad and the private fee payer | `msig-bench`, one build per pad size | executor only |
| bench/mac-cpu.log, bench/mac-NOTE.md | Apple M2 Max, CPU prover, clean run | `scripts/bench-mac.sh cpu` | real |
| bench/mac-metal.log | same machine, earlier loaded session; ran the CPU prover too (no Metal path in risc0 3.0.5) | `scripts/bench-mac.sh metal` | real |
| bench/real-run.log, bench/real-env.log | desktop RTX 3080: Create, fund, Execute at M = 5, 10, 30 | `scripts/bench-gpu.sh real` | real |
| bench/cycles.log, bench/cycles-env.log | executor cycles vs M (P.2) | `scripts/bench-gpu.sh cycles` | executor only |
| bench/dev-run.log, bench/dev-env.log | dev-mode negative control, earlier image build; not a benchmark | `scripts/bench-gpu.sh dev` | dev mode |
