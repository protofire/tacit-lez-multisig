# `multisig_module` API (RFP S.9)

Status: **BUILT, 2 calls** (`create`, `execute`), on guest image v2.1 (the module pins the same ELF; `evidence/module-headless-transcript.txt`). Propose, approve, reject, member and policy management, time lock and spending limit through the module are **PLANNED (M3)**; today they exist only in the `msig` CLI (`docs/CLI.md`), which drives the LEZ wallet directly, not through Logos core. Members are dev members (keys derived from public labels): not for real funds. Proofs in the transcripts are dev-mode. The `.lgx` is unsigned (`"signature":"unsigned"` at install); a signed package is PLANNED (M4).

Source: `modules/multisig_module/rust-lib/src/glue.rs` (trait), `core.rs` (logic), `metadata.json` (package: name `multisig_module`, version 0.1.0, no dependencies). Chain flow: `crates/msig-host/src/bin/msig-e2e.rs`.

## Conventions

- Every call returns **one JSON string**: success `{"ok":true, ...}`; failure `{"ok":false,"error":"<message>"}`. `logosctl call` succeeds whenever the RPC does; **check `ok` in the inner JSON** (`modules/headless/run.sh` does).
- `wallet_home`: a directory containing `wallet_config.json` (with the sequencer address). The module keeps its own `storage.json` and `msig.json` there. **One multisig per wallet home.**
- Numbers that can exceed 53 bits (`fund`, `amount`) are passed and returned as decimal strings.
- Calls are slow: each private tx is simulated, proven and awaited (set `LOGOSCTL_TIMEOUT`, the headless run uses 900000 ms).
- Dev-mode sequencer requires `RISC0_DEV_MODE=1` on the module side.

## `create(wallet_home, members, threshold, fund)`

| Arg | Type | Meaning |
|---|---|---|
| `wallet_home` | string | wallet directory, see above |
| `members` | int | N, 1..=255 at the module boundary; the program rejects N > 30 (`MAX_MEMBERS`, `crates/msig-core/src/lib.rs:20`) |
| `threshold` | int | M, `1 <= M <= N` |
| `fund` | string | native units credited to the vault, decimal u128 (`"0"` skips funding) |

Does: deploys the multisig program at `P` if absent (immutable), shields fee funds to the submitter's private account, creates the private multisig `V` (private PDA bound to `P`), credits `V` with `fund`.

Success:
```json
{"ok":true,"program":"<P>","vault":"<V>","createBlock":53,"fundBlock":63,
 "threshold":2,"members":3,"epoch":0,"nonce":0,"balance":"1000"}
```
Real run: `evidence/module-headless-transcript.txt`.

Errors (`error` text): `need 1 <= threshold <= members`; `members: 1..=255`; `fund: decimal u128`; `msig.json exists in the wallet home (...): one multisig per home`; `group 'msig-module' already in this wallet home ... refusing to replace it`; `...wallet_config.json: missing (sequencer address config)`; sequencer, deploy or tx errors, e.g. `<what>: tx <hash> REVERTED in block <n>`; `V balance <x> after funding, expected <y>`.

### Retry semantics of `create`
- It writes `msig.json` (`vault`, `submitter`, `funded`) right after `V` is derived and **before any tx touches V**.
- It **refuses to run** if `msig.json` or the `msig-module` group already exists. V's keys derive from the group secret, so replacing the group would strand a vault. After an interrupted `create`, do not retry in the same home: the earlier vault keeps its keys; use a fresh home or recover by hand.
- Evidence: RED `modules/headless/retry-red.log`, GREEN `retry-green.log` (scenario H1).

## `execute(wallet_home, recipient_label, amount, signers)`

| Arg | Type | Meaning |
|---|---|---|
| `wallet_home` | string | same home as `create` |
| `recipient_label` | string | dev recipient: a public account derived from this label |
| `amount` | string | native units, decimal u128 |
| `signers` | string | comma-separated dev member indices, e.g. `"1,2"`; member 0 always proposes and executes |

Does: opens the recipient account if needed (1 unit), then runs a quorum-approved native transfer from `V` to the recipient, and checks the balances.

Success:
```json
{"ok":true,"block":83,"approvals":2,"threshold":2,"nonce":1,
 "vaultBefore":"1000","vaultAfter":"900",
 "recipient":"<id>","recipientBefore":"1","recipientAfter":"101"}
```

Errors: `no multisig in this wallet home: run create first`; `amount: decimal u128`; `signers: e.g. 1,2`; program refusals surfaced from the guest exit code (below threshold, bad signature, not a member, ...; code map in `crates/msig-core/src/lib.rs` `Error`, e.g. 6 BelowThreshold, 8 NotMember, 10 BadSignature); `Execute: tx <hash> REVERTED ...`; `Transaction not found in preconfigured amount of blocks` (submitted, not confirmed: see retry); `unexpected balances after Execute`; `an earlier execute landed (pending ...); nothing sent now: call again for this payment`.

### Retry semantics of `execute`
- Before submitting, it stores the pending payment (signed `nonce`, `recipient`, `amount`, then the tx `hash`) in `msig.json`.
- The next call reconciles first:
  - nonce already consumed and the same payment: returns `{"ok":true,"reconciled":{...},"nonce":...,"vaultAfter":...,"recipientAfter":...}` and **signs nothing**;
  - nonce consumed but a different payment: refuses (`an earlier execute landed ...`); call again for the new payment;
  - nonce not consumed: signs again at the **same nonce**, so at most one of the two txs can land.
- So a caller may safely repeat the same `execute` after a timeout. Evidence: `modules/headless/retry-green.log` (scenario H2: second call returns `reconciled`; recipient ends at 401, not 701).

## Not exposed
propose, approve, reject, config changes, cancel, time lock (`queue`, `execute-queued`), spending limit (`spend`), reading the state: PLANNED (M3). Own catalog: PLANNED (M4). QML mini-app: PLANNED (B4). The `msig` CLI covers these operator flows today, except arbitrary calls and cancel: `docs/CLI.md`.

## Headless example

Through the Logos headless host (`modules/headless/run.sh`, run inside the Nix container by `modules/headless/headless.sh`):
```sh
logosctl daemon start --detach
logosctl install <path>/logos-multisig_module-module-lib.lgx -y
logosctl module load multisig_module
logosctl call multisig_module create  "$WALLET_HOME" 3 2 str:1000
logosctl call multisig_module execute "$WALLET_HOME" alice str:300 str:1,2
```
String-typed arguments are passed with the `str:` prefix, as in `run.sh`. Reproduce:
```sh
modules/headless/headless.sh                  # Logos host, needs Docker
MODE=direct modules/headless/headless.sh      # same flow through the Rust lib, no Logos host
MODE=retry  modules/headless/headless.sh      # retry scenarios H1 and H2
```
Needs a LEZ v0.4.0-rc1 sequencer binary (`LEZ_SEQUENCER_BIN`) and `MSIG_LEZ_PORT` of 3072 or above. See `modules/README.md`.

## Integration journey (another module calling this one)
1. Provide a wallet home with `wallet_config.json` pointing at a v0.4 sequencer.
2. Call `create` once; store `vault` and `program` from the result.
3. Call `execute` per payment; treat a result without `"ok":true` as failure; on a timeout, repeat the identical call.
4. Reading the multisig state, proposing and approving through the module are PLANNED (M3); until then use the CLI.
