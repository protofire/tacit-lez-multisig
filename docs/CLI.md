# `msig` operator CLI (RFP-005 U.3, U.6, S.10)

`msig` is the operator tool for the private M-of-N multisig. Operators never share a wallet:
each has their own wallet home, and **proposals travel as envelope files** (the chat room carries
the same files later). Status: BUILT, tested on a standalone dev-mode sequencer only
(`scripts/cli-walkthrough.sh`, `evidence/cli-walkthrough.txt`, image v2.1); real proving is
measured in `docs/BENCHMARKS.md`.

`msig` drives the patched LEZ wallet directly; it does not run through the Logos core headless
host. Only the module's `create` and `execute` run under `logosctl` (`docs/MODULE-API.md`). The
RFP's "CLI over the headless framework" (U.3) is therefore met for those two calls only; making
`msig` a front-end to `multisig_module` for every command below is PLANNED (M3).

## Install

```
git clone <repo> && cd <repo>
export PATH="$PATH:$HOME/.risc0/bin"      # risc0 toolchain (rzup), needed to build the guest
cargo build --release -p msig-cli         # binary: target/release/msig
```

Dev mode (`RISC0_DEV_MODE=1`) skips real proofs; without it `execute` produces real proofs, which
take minutes to hours on a CPU and are meant for a GPU box. Each operator sets
`LEE_WALLET_HOME_DIR=<dir>` to their own wallet home (holds `wallet_config.json` pointing at the
sequencer, `storage.json`, `msig-vault.json`, `msig-fee-account`).

## Concepts

- **Vault V**: a private PDA bound to the deployed multisig program. Its state (members, threshold,
  epoch, nonce) and balance are visible only to holders of the **group secret**.
- **Vault file** (`msig-vault.json`): group secret + seed + V. Written by `create`; give it to the
  other members out of band (it is a secret, mode 600) and they run `join`.
- **Member key file**: 32-byte secret, hex. `msig keygen` makes one and prints the public key.
  A member signs approvals with it (BIP-340). Dev-grade storage: plain file.
- **Fee account**: a private account each operator funds once (`setup`); it pays the fee of the
  private transactions that operator proves.

## Envelope format (version 1)

One JSON file:

```json
{ "version": 1,
  "signed": "<hex>",
  "sigs": [ { "role": "propose|approve|reject", "key": "<hex x-only pubkey>", "sig": "<hex 64B BIP-340>" } ] }
```

`signed` is the exact byte string the signatures cover: `V (32) ‖ body (56) ‖ borsh(action)` with
`body = epoch u64le ‖ nonce u64le ‖ expiry u64le ‖ sha256(borsh(action))` (ARCHITECTURE §2.4).
Signatures are over `D(tag, V ‖ body) = sha256(pad32(tag) ‖ V ‖ body)` with tags
`LEZ-MSIG/v1/propose`, `/approve`, and (advisory, never sent on chain) `/reject`. The executor's
`/execute` signature is made at `execute` time and is not stored in the file.

**Spend envelope** (image v2): `"kind": "spend"` and `signed = V (32) ‖ epoch u64le ‖ spend nonce
u64le ‖ p u64le ‖ to (32) ‖ amount u128le` (exactly 104 bytes), the program's `/spend` body.
Signatures have role `spend` over `D("LEZ-MSIG/v1/spend", V ‖ those 72 bytes)`; no proposer, no
executor signature. The same bytes read as an action envelope fail to decode, and a `spend`
signature never counts on an action envelope (nor the reverse).

`show`, `approve`, `reject` and `execute` decode everything from `signed` and **refuse** it if the
action hash inside `body` does not match the embedded action (`TAMPERED`). `show` re-verifies every
signature against the digest of `signed` and marks failures `INVALID` (non-zero exit).
Approving prints the decoded action first, so you sign what you saw.

## Commands

Global options: `--vault FILE` (default `$LEE_WALLET_HOME_DIR/msig-vault.json`), `--registry FILE`.

| Command | What it does |
|---|---|
| `keygen OUT` | Fresh member key to `OUT` (hex, 600); prints the public key. |
| `setup --payer-key F` | Init the wallet, deploy the multisig program at its address if absent (idempotent), create and fund this operator's private fee account from the public account `F` (dev/test funding path; any funded public key works). |
| `create --members PK1,PK2,.. --threshold M [--perms 7] [--chain-tag S]` | Generates group secret + salt, writes the vault file **before** submitting, proves and submits `Create`. Every member gets `--perms` (1 initiate, 2 vote, 4 execute, 8 spend; default 7). |
| `join FILE` | Import a vault file; the wallet then tracks V and decrypts its state. |
| `fund AMOUNT --payer-key F [--to-key K]` | Dev helper: public payer pays the vault (default) or opens/credits the public account of key file `K` (a recipient must exist publicly before a private tx can pay it). |
| `status` | Members (index, key, permission bits), threshold, epoch, nonce, balance, time lock, spending limit, spent in the current period, spend nonce, queue slot (eta), as this wallet decrypts them. |
| `propose --to ACCOUNT --amount N --key K --out FILE [--expiry-secs 3600]` | Envelope for a native transfer out of the vault, signed as proposer. Reads epoch and nonce from the vault after syncing. |
| `config add-member PK [--perms 7]` / `remove-member PK` / `set-threshold N` / `set-time-lock MS` / `set-limit AMOUNT --period-ms MS --threshold S` (each with `--key K --out FILE`) | Same, for a config change (needs the same quorum; bumps the epoch, which kills every earlier approval). |
| `spend --to ACCOUNT --amount N --key K --out FILE` | Spend envelope under the spending limit for the current period (`p = now / period`), signed by `K` (spend permission). Other spend members add theirs with `approve`; `execute` submits it. |
| `show FILE` | U.6: prints vault, epoch, nonce, expiry, action hash, approve digest, target program (via the registry), recipient, amount or config op, and each signature's validity. |
| `approve FILE --key K` | Prints the decoded action, then appends (or replaces) your approval in `FILE` (a `spend` signature on a spend envelope). |
| `reject FILE --key K` | Advisory rejection; `execute` ignores it and reports the count. |
| `execute FILE --key K` | Proves and submits `Execute` (below; `K` needs the execute permission), or `Spend` for a spend envelope. Under a time lock only a cancel executes immediately. |
| `queue FILE --key K` | Under a time lock: same quorum as `execute`; `K` must be the proposer (approvers do not sign `t`, so only the proposer picks it) and signs `/queue` with `t = now + 30 s`; proves and submits `Queue`. Prints the eta (`t + time lock`). |
| `execute-queued FILE --key K` | After the eta: checks the envelope is the queued action, `K` (execute permission) signs `/execute` over `body ‖ t`; proves and submits `ExecuteQueued`. Refused before eta (`TOO EARLY`) or for another action (`NOT QUEUED`). |

**Registry** (`--registry`): JSON `{"<program account id, base58>": "name"}`. Built in: the native
token program and the multisig program itself. A program not in the registry is shown as
`UNKNOWN program <id>` with the raw call data in hex. The registry maps a program **account**, not
an image. Only for this multisig program does the account follow from the image id (our own
deploy-key convention, `crates/msig-host/src/lib.rs:52-70`); for any other program the account says
nothing about its code. Decoding covers native-token messages and multisig actions only.
Registering a second program's IDL and decoding its calls is PLANNED (M2); registry pins signed by
k of N members PLANNED (M3).

## `execute` stages

```
[1/4] syncing wallet                     wallet synced right before the private spend
[2/4] vault at epoch E nonce N, balance B
[3/4] K valid approval(s) from current members (threshold M), R advisory rejection(s)
[4/4] proving and submitting
  [prove] still working, 25 s elapsed    printed every 5 s while proving
  [prove] done in 0.8 s
  [wait] for inclusion ...
executed. vault now: ...                 state re-read after the tx is included
```

`queue`, `execute-queued` and a spend `execute` print the same stages. Refused before any proving,
with a message: `STALE` (envelope epoch/nonce differ from the vault's; spend nonce for a spend),
`EXPIRED`, `BELOW THRESHOLD`, `TAMPERED`, wrong vault, missing initiate/execute permission. If the
program itself rejects the call, the guest error name is printed. Any other failure (typically a
stale wallet snapshot) triggers one re-sync and exactly one retry, with a message; a second failure
is reported.

Only the signatures of **current** members with the right permission are counted, sorted by member
index. The proposer's signature is separate from approvals: a proposer who also wants to count as
an approver runs `approve` too.

## Walkthrough: 2-of-3

`scripts/cli-walkthrough.sh` runs this on a fresh standalone sequencer (port 3081) and asserts the
outputs; transcript in `evidence/cli-walkthrough.txt`. By hand, with `$OPn` each operator's wallet
home and `msigN` meaning `LEE_WALLET_HOME_DIR=$OPn msig`:

```
# 0. keys (each operator keeps theirs), setup per operator (first one deploys)
PK1=$(msig1 keygen $OP1/member.key); PK2=$(msig2 keygen $OP2/member.key); PK3=$(msig3 keygen $OP3/member.key)
msig1 setup --payer-key payer.key; msig2 setup --payer-key payer.key; msig3 setup --payer-key payer.key

# 1. operator 1 creates the vault; the vault file goes to 2 and 3 over a private channel
msig1 create --members $PK1,$PK2,$PK3 --threshold 2
msig2 join $OP1/msig-vault.json;  msig3 join $OP1/msig-vault.json

# 2. fund (dev helper) and check
msig1 fund 1000 --payer-key payer.key;  msig2 status

# 3. propose a payout; 2 and 3 inspect and approve; the file moves between them
msig1 propose --to <recipient> --amount 300 --key $OP1/member.key --out prop.json
msig2 show prop.json                       # decoded from the signed bytes
msig2 approve prop.json --key $OP2/member.key
msig3 approve prop.json --key $OP3/member.key

# 4. anyone with the execute permission proves and submits
msig3 execute prop.json --key $OP3/member.key

# 5. remove member 3: same flow, quorum 2 of 3; the epoch moves, older approvals die
msig1 config remove-member $PK3 --key $OP1/member.key --out cfg.json
msig1 approve cfg.json --key $OP1/member.key;  msig2 approve cfg.json --key $OP2/member.key
msig2 execute cfg.json --key $OP2/member.key
msig1 execute prop-made-before-the-change.json --key $OP1/member.key    # -> STALE

# 6. (image v2) spending limit, then a Spend: one spend signature, no proposal
msig1 config set-limit 150 --period-ms 86400000 --threshold 1 --key $OP1/member.key --out lim.json
msig1 approve lim.json --key $OP1/member.key;  msig2 approve lim.json --key $OP2/member.key
msig2 execute lim.json --key $OP2/member.key
msig1 spend --to <recipient> --amount 100 --key $OP1/member.key --out spend.json
msig2 show spend.json;  msig2 execute spend.json --key $OP2/member.key

# 7. (image v2) time lock: propose + approve as usual, then queue, wait for eta, execute-queued
msig1 config set-time-lock 5000 ...       # same flow as 6.
msig1 propose --to <recipient> --amount 100 --key $OP1/member.key --out q.json   # + two approvals
msig1 queue q.json --key $OP1/member.key                  # prints eta
msig2 execute-queued q.json --key $OP2/member.key         # after eta
```

The walkthrough creates the vault with `--perms 15` so members can spend, and deploys the frozen
image v2.1 (`RISC0_USE_DOCKER=1` by default; `evidence/IMAGE-ID-v2.1.md`).

## Not in this prototype

| Missing | Plan |
|---|---|
| Executing arbitrary `Call` actions (`execute` accepts native transfers out of the vault, config and cancel only; `crates/msig-cli/src/main.rs:721-726`) | PLANNED (M2) |
| A `cancel` command to create a cancel envelope (the program supports `Cancel`) | PLANNED (M2) |
| Removing a spending limit (`SetLimit(None)`: `show` decodes it, no command makes it) | PLANNED (M2) |
| Sealed (`seal_for`) vault-file distribution; encrypted member key files | PLANNED (M2) |
| Joining with the viewing key only (narrows who can freeze the vault, `docs/ARCHITECTURE.md` §3.1) | PLANNED (M2) |
| Rounding `expiry` and `t` to a time grid (they are public fingerprints) | PLANNED (M2) |
| More than one queued action (the program has one slot; a live entry refuses a new `Queue` with `QueueOccupied`) | PLANNED (B1) |
| A rejection quorum (reject is advisory) | not planned |
| Hardware-wallet (Keycard) signing; keys are plain hex files today | not built; no milestone yet |
