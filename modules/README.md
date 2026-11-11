# modules/ — Logos module and headless run (RFP-005 U.1, U.3; PROTOTYPE)

`multisig_module` is a Logos core module with exactly two calls, both on a private M-of-N multisig
(dev keys only: payer, members and recipient derive from public labels):

| Call | Args | Does |
|---|---|---|
| `create` | `wallet_home members threshold fund` | deploys the multisig program at P if absent, shields fee funds to the submitter's private account, creates the private multisig V (shared private PDA bound to P), credits V `fund` native units |
| `execute` | `wallet_home recipient_label amount signers` | opens the dev recipient account if needed, runs a quorum-approved native transfer V -> recipient (`signers` = dev member indices, e.g. `1,2`; member 0 always proposes and executes) |

Both return a JSON string (`ok`, block ids, balances before and after, or `error`). Not exposed: propose,
approve, reject, config, policies, spending limits (M3). No QML UI (M4).

## Layout

- `multisig_module/rust-lib/src/core.rs` — plain Rust over `msig-core`, `msig-host` and the LEZ wallet; same flow as `crates/msig-host/src/bin/msig-e2e.rs` split in two calls that share a wallet home.
- `multisig_module/rust-lib/src/glue.rs` — Logos trait; the builder generates the plugin from it.
- `multisig_module/rust-lib/crates/msig-methods` — stand-in for `crates/msig-methods`: the committed guest build (`msig.bin`, image id checked by a test), because the Nix build has no risc0 guest toolchain. Pinned to the frozen image v2.1 (`evidence/IMAGE-ID-v2.1.md`): copy of the `RISC0_USE_DOCKER=1` build (`target/riscv-guest/.../docker/msig.bin`); replace it and the `MSIG_ID` words when the guest changes.
- `multisig_module/rust-lib/crates/msig-plumb` — empty stand-in for `crates/msig-plumb` (bench-only guest msig-host links for `msig-cycles`; never used by the module).
- `multisig_module/{flake.nix,metadata.json,CMakeLists.txt}` — module build (`logos-module-builder`); the flake stages real copies of `crates/msig-core`, `crates/msig-host` and `vendor/`.
- `nix-docker.sh` — Nix inside Docker (read-only lower volume `tacit-nix`, seeded once from the `nixos/nix` image; build cache in volume `tacit-nix-upper`; override with `NIX_LOWER_VOLUME` / `NIX_UPPER_VOLUME`). No Nix on the host.
- `headless/headless.sh` — host driver: fresh standalone LEZ v0.4.0-rc1 sequencer (dev mode, port 3072), wallet home, then the run, transcript to `evidence/module-headless-transcript.txt`.
- `headless/run.sh` — runs inside the container: `logosctl daemon`, install the module `.lgx`, `module load`, `call create`, `call execute`. The module declares no dependencies; if one is ever added, build and install it here first: a module whose dependencies are not installed does not load.

## Reproduce

```
modules/headless/headless.sh                 # through the Logos headless host (needs Docker; first run builds for a long time)
MODE=direct modules/headless/headless.sh     # fallback: the same flow through the Rust lib, no Logos host
MODE=retry OUT=modules/headless/retry-green.log modules/headless/headless.sh   # retry-safety scenarios (below)
```

Needs the sequencer binary of LEZ `1806e87` (`LEZ_SEQUENCER_BIN`; build it with `cargo build --release --locked -p sequencer_service --features standalone --manifest-path vendor/lez/Cargo.toml`),
`~/.risc0` and a host Rust toolchain for the one-off `msig-e2e` build (genesis key). Port: `MSIG_LEZ_PORT`, 3072 or above.
`RISC0_DEV_MODE=1`: proofs are dev-mode receipts, not real proofs.

## Evidence and limits

- `evidence/module-headless-transcript.txt`: `MODE=logos` run. The module `.lgx` built by Nix in Docker (aarch64-linux) loads in the `logosctl` headless host; `create` then `execute` complete on the sequencer (block ids and balances in the JSON results).
- Proofs are dev-mode receipts (`RISC0_DEV_MODE=1`); real-proof numbers are in `docs/BENCHMARKS.md`.
- The module runs image v2.1 (`e174a45f…34557`; the transcript header prints the id and P). It exposes only `create` and `execute` (immediate, time lock 0); Queue / Spend are reachable through the CLI, not the module. The in-tree guest (`rust-lib/crates/msig-methods/msig.bin`) is a copy: after any guest change, replace it and `MSIG_ID` (the test `image_id_matches_elf` fails otherwise).
- Build workarounds, all in `multisig_module/`: `third_party/rapidsnark` (copy of the vendored crate that carries `RAPIDSNARK_VERSION`, plus a prebuilt-lib hook; the flake fetches the aarch64 zip), `LINK_LIBRARIES pcsclite` in `CMakeLists.txt` (the wallet pulls `pcsc-sys`, whose `SCard*` symbols the staticlib leaves undefined), `lee/prove` (no `r0vm` on the Linux host). x86_64-linux needs its own rapidsnark zip hash in `flake.nix`.
- `logos-rust-sdk-src` is a gitignored local stub so `cargo build --no-default-features` works on the host; the Logos builder supplies the real SDK. `rust-lib/Cargo.lock` therefore carries the stub's `logos-rust-sdk 0.0.0` entry; the builder rewrites it.
- Retry safety (scenarios H1/H2; RED at the old code `modules/headless/retry-red.log`, GREEN `retry-green.log`): `create` writes `msig.json` (vault, submitter, `funded`) right after V is derived, before any tx touches V, and refuses to run when `msig.json` or the `msig-module` group already exists, so a retried `create` never replaces the group secret V's keys derive from. `execute` records the pending payment (signed nonce, recipient, amount, then tx hash) in `msig.json` before submitting; the next call reconciles it first: nonce consumed -> reports the earlier payment (`reconciled`) and signs nothing; not consumed -> signs again at the same nonce, so at most one of the two lands.
- One multisig per wallet home; dev members only (indices from `dev_cfg`); no catalog (M4); darwin module build not done.
