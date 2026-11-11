# What the note pad and the private fee payer cost in proved cycles (LEZ v0.4.0-rc1, image v2.1)

LEZ vendor 1806e87, risc0-zkvm 3.0.5, msig image
e174a45fc81ca8ba92f8b407825546fbb38377c10dc8d4362204266efc134557 (asserted by msig-bench on every run).
Executor only (`RISC0_DEV_MODE=1`): exact cycle counts, no proving. Method as in `evidence/tx-cycles.md`.

## Table

Wallet path of `scripts/bench-mac.sh` (7-slot private padding, private fee payer, Undisclosed claim).
Outer = privacy circuit session; inner = msig program session that gets proved; "padded" = risc0
`total cycles` (sum of 2^po2 over segments). CPU s = total padded x 95 s per million (M2 Max CPU fit
from tx-cycles.md), an estimate, not a measurement. tx bytes are dev-mode (fake receipt that still
carries the journal); add ~223 KB for a real succinct seal (real proof 267,166 B vs dev 44,153 B at 4096).

| pad | op | outer user | outer segs (padded) | inner padded | total padded | tx bytes (dev) | est. Mac CPU s |
|---|---|---|---|---|---|---|---|
| 512 (upstream) | fund | 3,813,833 | 5 (4.46M) | 0 | 4.46M | 31,729 | 423 |
| 512 | Create M=5 | 3,787,162 | 5 (4.46M) | 0.52M | 4.98M | 30,849 | 473 |
| 512 | Execute M=5 | 3,886,807 | 5 (4.72M) | 2.10M | 6.82M | 31,353 | 647 |
| 512 | Create M=10 | 3,850,648 | 5 (4.72M) | 0.52M | 5.24M | 31,115 | 498 |
| 512 | Execute M=10 | 3,916,493 | 5 (4.72M) | 3.67M | 8.39M | 31,683 | 797 |
| 2048 | fund | 4,264,828 | 5 (5.24M) | 0 | 5.24M | 56,265 | 498 |
| 2048 | Create M=5 | 4,251,790 | 5 (5.24M) | 0.52M | 5.77M | 55,425 | 548 |
| 2048 | Execute M=5 | 4,341,113 | 5 (5.24M) | 2.10M | 7.34M | 55,889 | 697 |
| 2048 | Create M=10 | 4,302,783 | 5 (5.24M) | 0.52M | 5.77M | 55,425 | 548 |
| 2048 | Execute M=10 | 4,363,424 | 5 (5.24M) | 3.67M | 8.91M | 55,889 | 847 |
| 4096 (ours, patch #2) | fund | 4,870,383 | 6 (5.77M) | 0 | 5.77M | 89,033 | 548 |
| 4096 | Create M=5 | 4,867,879 | 6 (5.77M) | 0.52M | 6.29M | 88,193 | 598 |
| 4096 | Execute M=5 | 4,953,355 | 6 (5.77M) | 2.10M | 7.86M | 88,657 | 747 |
| 4096 | Create M=10 | 4,923,413 | 6 (5.77M) | 0.52M | 6.29M | 88,193 | 598 |
| 4096 | Execute M=10 | 4,984,088 | 6 (5.77M) | 3.67M | 9.44M | 88,657 | 897 |

fund at M=10 equals M=5 within 0.1 % (4096: 4,873,764; 2048: 4,265,290; 512: 3,837,375). 4096 rows
match tx-cycles.md (e.g. fund 4,858,960 there, Execute M=5 4,941,823); the inner session does not
depend on the pad (Execute M=5 1.92M user in all three).

- Pad cost is linear: ~296 outer user cycles per pad byte per tx (8 padded notes, 7 actions + the
  fee fallback, ~37 cycles/byte: ChaCha20 keystream + hashing), 4096 -> 2048 saves ~606K user cycles.
- In padded terms 4096 -> 2048 drops the outer session from 6 to 5 segments: -524,288 padded
  (one 2^19 segment) on every private tx, ~50 s of Mac CPU, 7 % of Execute M=5, 9 % of fund/Create.
  2048 -> 512 saves another 0.52-0.79M padded (~50-75 s) but leaks N (below).
- tx bytes: -32,768 B per tx from 4096 to 2048 (8 notes x 2048 B, twice: in the message and in the
  receipt journal).

Notes observed on chain (msig-bench `NOTES` line, every private action of the tx):
4096 and 2048 = all 7 notes exactly the pad, every op. 512 = V's note leaks:
532 B (N=7 after fund), 645 B (N=12 Create), 697 B (N=12 funded).

## Does 2048 hide N up to MAX_MEMBERS = 30? Yes, for every op.

- A note is `PrivateAccountKind::HEADER_LEN` (65, `vendor/lez/lee/state_machine/core/src/program/mod.rs:132`)
  + borsh `Account` (`encoding.rs:19`), zero-padded up to the pad and XORed with a ChaCha20 keystream
  (`encryption/mod.rs:104`, `:176`, `:189`); stream cipher, no tag, so ciphertext = plaintext length.
  The ML-KEM epk (1088 B) is a separate fixed-size field. The wallet uses the same formula for its
  overflow warning (`lez/wallet/src/account_manager.rs:1327` `note_plaintext_len`, `:687`).
- V's account holds two actor states: native balance (16 B) and the msig `Multisig` (`crates/msig-core/src/lib.rs:85`).
  Approvals and signatures are instruction data, never state, so the note size depends only on the
  state layout, and the largest state is the same for every op (Create, Execute, Queue,
  ExecuteQueued, Spend, Config).
- Worst case, computed with borsh (scratch test, not committed): N = 30, `limit`, `queued` set,
  all integers max: state 1,191 B, account 1,299 B, note plaintext **1,364 B** = 65 + 16 nonce
  + 4 map len + (32+4+16) balance + (32+4+1,191) msig. Headroom at 2048: 684 B. (Earlier 1,315 B
  at N=32 was the v1 state layout; same formula: 173 + state.)
- Measured: 2048 run, N = 7 and N = 12, all ops: every note 2048 B.
- Limit: an extra actor state on V (another token program) adds 36 + its size; 2048 leaves 684 B for
  that. N > 50 would not fit (each member 33 B, note = 374 + 33 N).

## Private vs public fee payer

Not measurable: a public fee payer for a private tx does not exist in rc1.
- Protocol: the sequencer charges a private tx only via a `PrivateToPublic` crossing into the fee
  inbox (`vendor/lez/lez/chain_state/src/classify.rs:68` `private_fee_reserve`); without it the tx is
  rejected with `MissingPrivateFee` (`classify.rs:120-121`).
- Wallet: payer = named payer, else `set_private_fee_payer`, else an owned private account
  (`lez/wallet/src/lib.rs:1532`, `account_manager.rs:757`); none -> "Privacy-preserving transaction has
  no private account to pay its fee" (`lib.rs:1689`).
- What the private payer occupies: one of the 7 padded private slots that would otherwise hold a dummy,
  plus a native fee transfer the circuit recomputes (no inner proof). Its isolated cost was not
  measured; it needs new wallet code to run a tx without the fee entry, which the sequencer rejects anyway.

## Method

Diagnostic harness tweak (not in this tree): msig-bench prints the note lengths
of every private action (`NOTES ... pad= lens=`) instead of asserting `== msig_core::NOTE_PAD`.
One target dir, built with Docker (guest image asserted v2.1) and the macOS 27 Metal fix +
RECURSION_SRC_PATH of `scripts/bench-mac.sh`; per variant `CIPHERTEXT_PAD_SIZE` edited in
`vendor/lez/lez/wallet/src/account_manager.rs:37`, rebuilt (cargo recompiled wallet + msig-host),
binary copied aside (three distinct sha1s), vendor restored. The NOTES line in each run proves which
pad the binary used. Per variant a fresh `sequencer_service` (LEZ 1806e87) on 127.0.0.1:3090,
`RISC0_DEV_MODE=1 BENCH_MODE=dev RISC0_INFO=1 RUST_LOG=risc0_zkvm=info`, then
`msig-bench init; create 5 7 c5; fund c5 100000; exec c5; create 10 12 c10; fund c10 100000; exec c10`.
Outer = the session with most user cycles in the op's log (one per op); inner = the last msig session.

## Recommendation

1. Change patch #2 to 2048: hides N for every op up to N = 30 (max note 1,364 B), one outer segment
   fewer on every private tx (-0.52M padded, ~50 s Mac CPU, 7-9 %), -32.8 KB per tx.
2. Keep 512 off the table: it leaks N from N = 7 (funded V note 532 B).
3. Keep the private fee payer: rc1 has no public-payer path for private txs (protocol rule), so there
   is nothing to trade.
