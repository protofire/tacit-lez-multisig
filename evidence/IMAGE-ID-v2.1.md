# Image v2.1 (frozen): multisig guest `MSIG_ID`

```
msig e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557
```

Built with `scripts/image-ids.sh`: `RISC0_USE_DOCKER=1`, risc0 reproducible builder
`risczero/risc0-guest-builder:r0.1.97.0@sha256:6f6c300e…62fb` (linux/amd64, pinned in
`crates/msig-methods/build.rs`), risc0-build 3.0.5, `--locked`. Changes from image v2, all in
`crates/msig-core/src/lib.rs`:

- `validate` rejects `time_lock > MAX_TIME_LOCK` (30 days in ms), so a time lock can no longer
  make every later `Queue` impossible.
- `SetLimit` resets `spend.period_index` and `spend.spent` (kept: `spend.nonce`), so a period
  change (daily to weekly) does not leave the counter in the old unit.
- `Queue` requires `queuer.index == proposer.index` (new error `QueuerNotProposer = 22`):
  approvers do not sign `t`, so only the proposer chooses it.

Digest layouts and the state layout (`VERSION = 2`) are unchanged; the digest known-answer tests
(`approval_digests_known_answers`, `queue_and_spend_digests_known_answers`) hold.

| Machine | Output | Wall |
|---|---|---|
| Mac (Darwin arm64, Docker 29.9.2, amd64 builder under emulation) | `msig e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557` | 1 m 51 s |

`IMAGE_ID_V2_1` in `crates/msig-host/src/lib.rs`; `msig-bench`, `msig-cycles` and `msig-prove`
assert it at runtime. The module pins the same ELF (`modules/multisig_module/rust-lib/crates/msig-methods`).

Guest inputs, Mac:
```
7700c55230c7420c1878400edb86e32b365f9d94d0f8754086006863a768b6a8  crates/msig-core/src/lib.rs
113ca628b03791f05bf4b3a57b4bd282e3b887aa8636d7122872d76c18ef9c42  crates/msig-methods/msig-guest/src/bin/msig.rs
9f52dbefbfb5cbad3c6ec0d94ed56de76344a2b7714f6cfa7d525b0edeadd751  crates/msig-methods/msig-guest/src/bin/noop.rs
935c29f51d473481f214965f2d1cb3a8b4f34e5bc83bdd5dfcf62470b52357a0  crates/msig-methods/msig-guest/Cargo.lock
81db016ae151a165bdc18acfe8aad2f7d87c0e424ed0c2a974c7f48246977142  crates/msig-methods/msig-guest/Cargo.toml
16c89e539d5bb49de67459296ef2c255ea2685857a2b19e058fb4c488f5dba9e  crates/msig-methods/build.rs
```

Reproduce from a checkout of this repository: `sh scripts/image-ids.sh` (Docker needed). The first line it prints must be
`msig e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557`; CI does the same comparison (job `image-id`).
