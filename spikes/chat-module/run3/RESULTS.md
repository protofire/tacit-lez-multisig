# Spike run 3: 2-of-3 coordination room on the Logos chat module, three members, real msig approvals

Apple Silicon Mac, native macOS. One scripted run, `room.sh`, 19:59:16 to 20:10:59 (11.7 min), result `PASS`.

**Dev-mode proofs** (`RISC0_DEV_MODE=1`) on a standalone LEZ v0.4.0-rc1 sequencer at 127.0.0.1:3085.
**Chat network: `logos.dev`** (no RLN). We did not use `logos.test`: it needs an RLN membership (found in an earlier 2-member spike, run 2).

## Verdict
A proposal was deliberated and approved in the room and then executed on chain. A proposed into a group conversation. B and C took the exact bytes from the room, decoded them with `msig show`, approved with their own member keys and posted the approvals back. A validated each approval, dropped the forged one, merged the rest and executed. The vault went 1000 → 700 and the recipient's public balance went 1 → 301 (sequencer RPC `getAccountBalance`).

| Step | Crossed | Evidence |
|---|---|---|
| 1. Three wallets + 2-of-3 private vault funded with 1000 | yes | a.log `create`, b.log `status` asserts `balance 1000` |
| 1. Three logosctl daemons, chat_module online on logos.dev | yes | room.log: each online in about 2.2 s |
| 2. A creates the room and adds B and C, both join | yes | joined 62.5 s (B) and 62.6 s (C) after `add_group_member` |
| 3. A proposes and posts `msig1:prop:`+base64(envelope) | yes | 728 B envelope, 983 chars on the wire |
| 4. B and C receive it, `show` asserts recipient/amount, approve, post back | yes | b.log/c.log: `received bytes identical to A's file`, room sender = A's address, ASSERTs |
| 4. B posts a forged approval | yes (it crossed the room) | a.log room message 2.txt from B's address |
| 5. A collects 3 approvals, drops the forged one, merges, executes | yes | a.log: 2 valid, 1 `INVALID`, `executed`, `balance 700`, `nonce 1` |
| 6. Async: C offline during proposal 2 + B's approval | C did NOT receive it (see below) | c.log after the restart |

## Forged approval
B's real member key signed `approve` over **different bytes**: B's own proposal of 999 to the same recipient. That signature entry was grafted onto the real envelope and posted to the room as `msig1:approve:` (forge code in `room.sh`).
- `msig show` on proposal + forged signature: `approve 9557be29… INVALID`, non-zero exit. A dropped it.
- `msig execute` on proposal + forged B + valid C: `[3/4] 1 valid approval(s) … (threshold 2)` → `BELOW THRESHOLD`, refused before proving. So the forged signature counts for nothing even when it is left in the file.
- The program check is not exercised in this run. The CLI filters the forged signature before anything is proved, so the guest never sees it. The in-guest rejection of a bad or non-member signature is covered by the msig-host tests listed in docs/ADVERSARY.md row "Inject a forged proposal or approval".
- Not tested: a valid signature from a **non-member** key. `show` checks signatures, not membership, so it would probably print `valid`. `execute` would not count it (it only counts current members).

## Async check (step 6)
C's daemon was stopped (`daemon stop`) before A posted proposal 2 (amount 50). B received it in 2.1 s, approved and posted. Then C restarted (`daemon start`, `module load`, `init` logos.dev; online in 0.1 s):
- C has a **new chat address** (`e5436f16…` before, `7d1e4a58…` after).
- The room is back as `history_only:true`, `message_count:4`. Those are the 4 messages from before the stop: the proposal, both approvals and the forged one.
- C got **no** `msig1:prop2:` in 180 s of polling. Messages sent while a member is offline are lost to that member. C would need to be re-added under its new address, and that would not replay the missed messages.
- Liveness did not depend on C: A approved proposal 2 itself, took B's approval from the room and executed. Vault 700 → 650, recipient 351.

## Timings (single run, one machine)
| What | Measured |
|---|---|
| Chain setup (sequencer, 3× `setup`, `create`, joins, 2 funds) | 5 min 7 s (19:59:18 → 20:04:25) |
| Daemon start + install by path + load, 3 members | ~55 s total (20:04:25 → first online 20:05:20) |
| `init` → `online` | 2.1-2.2 s each |
| `add_group_member` → member sees room | 62.5 s B, 62.6 s C (the earlier 2-member run 2: ~91 s) |
| Proposal post → B / C have it | first poll 2.1 s (B), 0 s (C polled after B's work) |
| Proposal post → all 3 approvals at A | 5.5 s (includes B/C show + approve) |
| Message sizes | 983-1339 chars per post, well under the 153,600 B cap |
| `execute` dev-mode prove | 0.9 s; through to inclusion ~33 s |

## Exact commands
`ctl.sh W …` = `HOME=run3/home/W $LOGOSCTL --config-dir run3/home/W/session …`. `msig` = `LEE_WALLET_HOME_DIR=run3/w/W target/debug/msig`. Every call with its output is in `a.log`, `b.log` and `c.log`. Order:
1. Sequencer `setup` + `start --rpc-listen-address 127.0.0.1:3085`, same config as `scripts/cli-walkthrough.sh`. Then `msig keygen` ×3, `setup` ×3, `create --members A,B,C --threshold 2`, `join` (B, C), `fund 1000`, `fund 1 --to-key recipient`.
2. Per member: `daemon start --detach`, `install $LGX_DIR/delivery_module-0.3.0.lgx -y`, `install $LGX_DIR/chat_module-0.3.0.lgx -y`, `module load chat_module`, `watch chat_module`, `call chat_module init 'json:{"delivery_preset":"logos.dev","log_level":"info"}'`, poll `status`, `get_address`.
3. A: `create_group_conversation msig-room run3`, `add_group_member G <B>`, `add_group_member G <C>`. B and C poll `list_conversations`.
4. A: `msig propose --to R --amount 300`, then `send_message G msig1:prop:<b64>`. B and C: `get_messages G`, base64-decode, `msig show`, `msig approve`, `send_message G msig1:approve:<b64>`. B forges, then posts it the same way.
5. A: `get_messages G`. For each approval, check that `signed` equals the proposal's, then `msig show` (proposal + that approval's sigs). Merge the valid ones, then `msig execute merged.json`.
6. C: `daemon stop`. A/B round 2 with `msig1:prop2:` and `msig1:approve2:`. C: `daemon start`, `module load`, `init`, poll for 180 s. A: `approve`, `execute`.

The merge is about 10 lines of Python in `room.sh`. The CLI has no merge command: each approver posts its whole approved envelope and A takes the union of the `approve` signatures.

## Versions
logosctl 0.3.2 (release binary), chat_module 0.3.0 + delivery_module 0.3.0 release lgx installed by path, msig CLI `target/debug/msig` (`cargo build -p msig-cli`; the run used an earlier revision of this code with the same envelope format, see the first line of `room.log`), LEZ sequencer_service v0.4.0-rc1 1806e87.

## Prerequisites (to re-run)
- `cargo build -p msig-cli` at the repository root.
- `LEZ_SEQUENCER_BIN`: `sequencer_service` built from LEZ 1806e87 (`cargo build --release --locked -p sequencer_service --features standalone --manifest-path vendor/lez/Cargo.toml`). Port 3085 must be free.
- `LOGOSCTL`: `gh release download 0.3.2 -R logos-co/logos-logoscore-cli -p logosctl-aarch64-macos.tar.gz`, `tar xzf`, `xattr -dr com.apple.quarantine .`, then point `LOGOSCTL` at `logosctl-aarch64-macos/bin/logosctl`.
- `LGX_DIR`: a directory with the release files `chat_module-0.3.0.lgx` (sha256 `4bf1b4ce8067f6dbab48355ae62319ed3e70b56a24e38049ecf138d420324425`) and `delivery_module-0.3.0.lgx` (sha256 `f744f0f9ef84438b6da3985561cb8d2eedf36c9340d29d839353cf8d728aa088`) from `github.com/logos-co/logos-modules-release` (releases `chat_module-v0.3.0`, `delivery_module-v0.3.0`). Installing from the catalog (`logosctl install chat_module`) hung in our runs; install by path.

## Shortcuts and limits
- The vault file (group secret) was handed to B and C on disk, not through the room. Sealed distribution is out of scope.
- One run, one machine, all three daemons on the same host and network.
- The room has no ordering or ack. `send_message` success does not mean delivered (run 2).
- Processes: on exit, `cleanup` stops all three daemons and kills the sequencer. Afterwards `pgrep -fl 'logosctl|logos_host|sequencer_service'` shows none of this run's processes, and nothing listens on :3085.

## Files
`room.sh`, `ctl.sh`, `room.log` (timeline), `a.log`, `b.log`, `c.log` (per-member transcripts). `home/`, `w/`, `net/` and `out/` are gitignored: they hold daemon sessions, wallets and keys (dev only), the sequencer and raw events.
