# Tacit — private M-of-N multisig on LEZ v0.4 (prototype)

A multisig program for the Logos Execution Zone (LEZ v0.4.0-rc1, `1806e87`). The default posture is a private PDA whose keys come from a Group Master Secret (GMS). Approvals are BIP-340 signatures collected off-chain and verified inside the program.

The built design is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). It covers guest image v2.1, `e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557`, deployed at program account `P = 2yigtJM3BuZ5TPsVKtB7DU1VewL7rn7R6Sz8DsrVBf7a` (`evidence/IMAGE-ID-v2.1.md`).

This is a prototype:
- dev keys;
- dev-mode proofs in every committed end-to-end transcript, with real proofs only in the benchmarks;
- a local standalone sequencer only;
- not audited and not for real funds.

**Known risk: any GMS holder, including a removed member, can freeze the vault's funds** (ARCHITECTURE §3.1). The freeze is reproduced by a test and is not fixed. Its decision gate is in M1.

## What is built

- Program instructions `Create`, `Execute`, `Config` (add, remove, threshold, time lock, limit), `Cancel`, `Queue` / `ExecuteQueued` (single slot) and `Spend`, with role bits, nonce and config epoch (`crates/msig-core`).
- Private posture end to end on a standalone LEZ v0.4.0-rc1 sequencer, at the deployed program account `P` (`scripts/e2e-private.sh`). The public posture is tested at handler level only.
- Tests for quorum, roles, replay, staleness, init safety, the transaction byte scan and viewing-key decryption (`crates/*/tests`).
- The `msig` operator CLI over signed envelope files with decoded signing ([docs/CLI.md](docs/CLI.md)).
- A Logos module with `create` and `execute`, run headless under `logosctl` ([docs/MODULE-API.md](docs/MODULE-API.md)).
- A SPEL-format IDL with a drift check (`idl/`), real-proof benchmarks ([docs/BENCHMARKS.md](docs/BENCHMARKS.md)) and a chat-module room spike.

Privacy per item: [docs/PRIVACY.md](docs/PRIVACY.md). Room adversaries: [docs/ADVERSARY.md](docs/ADVERSARY.md). Decision log: [docs/DECISIONS.md](docs/DECISIONS.md). Requirement-by-requirement status: `proposal/COVERAGE.md`.

## Prerequisites

- Rust: the toolchain in `rust-toolchain.toml` (1.98.1; `rustup` picks it up).
- The RISC Zero toolchain in `~/.risc0`: `curl -L https://risczero.com/install | bash`, then `rzup install rust 1.97.0`, `rzup install cpp 2024.1.5`, `rzup install cargo-risczero 3.0.5`, `rzup install r0vm 3.0.5` (as `.github/actions/setup/action.yml`).
- On Linux: `pkg-config`, `libssl-dev`, `libpcsclite-dev` (the LEZ wallet links PC/SC), `lsof` (used by the scripts), `clang`, `cmake`.
- Docker, for the reproducible guest build (`RISC0_USE_DOCKER=1`) and the Logos module build.
- A LEZ v0.4.0-rc1 `sequencer_service` in `LEZ_SEQUENCER_BIN`, built from the vendored tree:

```sh
cargo build --release --locked -p sequencer_service --features standalone --manifest-path vendor/lez/Cargo.toml
export LEZ_SEQUENCER_BIN=$PWD/vendor/lez/target/release/sequencer_service
```

`--features standalone` runs the sequencer without a Logos blockchain node (mock Bedrock). Without it the sequencer waits for a node on `localhost:18080` and never opens its RPC port.

The scripts start their own sequencer on ports 3070 and above.

**macOS 27 only:** the Metal compiler rejects `cells[i] = 0` on `Fp` in risc0-sys 1.5.0 (`poseidon2.metal`). If a host build fails there, build against a patched copy, as `scripts/bench-mac.sh` does: copy `~/.cargo/registry/src/*/risc0-sys-1.5.0` to `target/risc0-sys-metal-fix`, replace `] = 0;` with `] = Fp(0);` in `kernels/zkp/metal/poseidon2.metal`, and add `--config 'patch.crates-io.risc0-sys.path="target/risc0-sys-metal-fix"'` to the cargo command (then `git checkout Cargo.lock`). The guest image is not affected.

## Verify the image id

```sh
scripts/image-ids.sh        # needs Docker; prints: msig e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557
```

The guest is built in risc0's reproducible Docker builder, pinned by digest in `crates/msig-methods/build.rs`, so the id does not depend on the host, the local toolchain or the checkout path. CI's `image-id` job runs the same build and compares it with `evidence/IMAGE-ID-v2.1.md`. Without `RISC0_USE_DOCKER=1` the id embeds host paths and differs per machine. `P` derives from the image id.

## Build, test, run

```sh
# Tests (dev mode: guests execute, proofs are not generated)
RISC0_USE_DOCKER=1 RISC0_DEV_MODE=1 cargo test --workspace
python3 scripts/gen_idl.py --check               # IDL in sync with the Rust sources

# End-to-end on a fresh standalone sequencer (dev mode): deploy at P, create, fund, execute, rejections
RISC0_USE_DOCKER=1 scripts/e2e-private.sh        # writes evidence/e2e-private-transcript.txt; ends with PASS

# CLI walkthrough, 3 operators (create, propose, approve, execute, remove member, limit + Spend, time lock + Queue)
scripts/cli-walkthrough.sh                       # writes evidence/cli-walkthrough.txt

# Logos module, headless
modules/headless/headless.sh                     # through logosctl, Nix in Docker (long first build)
MODE=direct modules/headless/headless.sh         # same flow via the Rust lib, no Logos host
MODE=retry  modules/headless/headless.sh         # retry-safety scenarios
```

Each transcript's first line records the guest image id and the LEZ version it ran against.

## Real-proof benchmark

`RISC0_DEV_MODE` must be unset; the sequencer then verifies every receipt. Both scripts need `RISC0_USE_DOCKER=1` (the bench binaries refuse any image other than v2.1) and `LEZ_SEQUENCER_BIN`.

```sh
RISC0_USE_DOCKER=1 scripts/bench-gpu.sh real     # Linux + CUDA: Create, fund, Execute at M = 5, 10, 30
RISC0_USE_DOCKER=1 scripts/bench-mac.sh cpu      # macOS, CPU prover: Execute at M = 5, 10 (applies the macOS 27 fix itself)
```

`scripts/real-proof.sh <msig-bench args>` runs one measured operation (wall time, peak RSS, peak GPU memory); `bench-gpu.sh` drives it. Method and results: [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

## Coordination today

Proposals and approvals travel as signed envelope files (`docs/CLI.md`). The room on the Logos chat module is not built.

Spike run 3 carried a real proposal and two approvals through a 3-member chat-module group on `logos.dev`. The executor dropped a forged approval, and the transfer executed. A member who was offline during a later proposal never received it: after restart it had a new chat address. Details: ARCHITECTURE §4.1 and `spikes/chat-module/run3/RESULTS.md`.

## Evidence index

| File | Shows |
|---|---|
| `evidence/IMAGE-ID-v2.1.md` | frozen guest image v2.1 and how to reproduce it |
| `evidence/e2e-private-transcript.txt` | create, fund, execute, below-quorum rejection, replay, removal, stale approvals (dev mode, image v2.1) |
| `evidence/cli-walkthrough.txt` | 3-operator CLI flow incl. limit, `Spend`, time lock, `Queue` → `ExecuteQueued` (dev mode, image v2.1) |
| `evidence/module-headless-transcript.txt` | module `create` and `execute` through `logosctl` (image v2.1) |
| `evidence/cargo-test-workspace.txt` | workspace test log |
| `evidence/leak-surface.md` | per-field content of a private `Execute` |
| `evidence/tx-cycles.md`, `evidence/pad-fee-cost.md` | where the proving cycles go; cost of the note pad and the private fee payer |
| `evidence/bench/` | real-proof logs: desktop GPU and Mac CPU (image v2.1), cycle counts |
| `modules/headless/retry-red.log`, `retry-green.log` | module retry safety, before and after the fix |
| `spikes/chat-module/run3/RESULTS.md` | chat-module spike: 3 members with real approvals |

See also `evidence/README.md`.

## Layout

| Path | Contents |
|---|---|
| `crates/msig-core` | types, digests, handler; guest-side |
| `crates/msig-methods` | guest build |
| `crates/msig-host` | tx builders, test harness, e2e and bench runners |
| `crates/msig-cli` | `msig` |
| `modules/` | Logos module and headless |
| `idl/` | SPEL-format IDL and its check |
| `proposal/` | milestones and requirement coverage |
| `vendor/lez` | LEZ with 2 wallet patches: `vendor/PATCHES.md` |
| `scripts/` | e2e, CLI walkthrough, benchmarks, image id, IDL generator |
| `docs/` | architecture, privacy, adversaries, CLI, module API, benchmarks, decision log, UI mock |
| `evidence/` | transcripts, logs and measurements |
| `spikes/` | chat-module coordination-room spike |

## Licence

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
