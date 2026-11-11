# Privacy: who sees what (RFP U.5, Pr.1–Pr.3)

**Mechanisms.** `docs/ARCHITECTURE.md` §3 describes them.

**Status tags.**
- **BUILT**: a test or evidence file is cited.
- **PLANNED (Mx/Bx)**: not built.
- **NOT MET**.

**Evidence base.**
- All committed transcripts use dev-mode receipts (`RISC0_DEV_MODE=1`). Real-proof runs are in `evidence/bench/` and `docs/BENCHMARKS.md`.
- Tests named below live in two files:
  - `crates/msig-host/tests/private.rs`: chain level, in-process LEZ state machine;
  - `crates/msig-core/tests/handler.rs`: program level.

**Audiences.**
- **Outsider**: a chain or transport observer.
- **Member**: holds the Group Master Secret (GMS).
- **Removed member**: still holds the GMS.
- **Viewing-key holder**: a disclosure audience.
- **Sequencer**: sees every submitted transaction, but not its plaintext.

## 1. The ten items, private posture (default)

| # | Item | Outsider | Member | Removed member | Sequencer | Enforced by | Evidence | Status |
|---|---|---|---|---|---|---|---|---|
| 1 | Existence of the multisig | The transaction does not name `V` or the program. Each transaction shows only a nullifier, a commitment and 4096-byte notes. The transaction itself is visible and is distinguishable as this wallet's by pad class and size (section 3). | visible | visible (keeps the GMS) | sees a private transaction | private PDA; `Undisclosed` program claim | `private_tx_byte_scan`, `evidence/leak-surface.md` | BUILT |
| 2 | Member set | hidden | visible | visible until re-key | hidden | signatures verified inside the private transition; private fee payer; `refund_to = None` | `private_tx_byte_scan` (no member key or account id in the tx bytes, N = 3 and N = 30) | BUILT |
| 3 | Threshold, roles, time lock, limit | hidden | visible | visible until re-key | hidden | note padded to `NOTE_PAD = 4096` (`crates/msig-core/src/lib.rs:41`) | `private_tx_byte_scan` (note length constant) | BUILT |
| 4 | Per-signer approval attribution | hidden: the private root message is not published | The state keeps no approver record. Signatures are visible to whoever holds the envelope. | same as member, for envelopes they saw | hidden | in-guest verification | `approvals_are_attributed_to_the_member_at_their_index`, `private_tx_byte_scan` | BUILT. An on-chain approver record (`last`) is not built and not in a milestone |
| 5 | Pending-proposal metadata | not on chain | in envelope files | files they received | not seen | approvals are collected off-chain | e2e steps 5–7 build approvals locally | BUILT with files; room transport PLANNED (M3) |
| 6 | Action payload | hidden while pending; at execution only the public effects | visible | visible until re-key | public effects only | the action runs inside the private transition | `private_tx_byte_scan`, `evidence/leak-surface.md` rows `boundary`, `public_root` | BUILT |
| 7 | Vault balance | hidden | visible | visible until re-key | hidden | encrypted state note | `viewing_key_alone_decrypts_state`, `quorum_execute_moves_native_balance_out_of_v` | BUILT |
| 8 | Execution linkage | Successive states of `V` are unlinkable by nullifier and commitment. They are linkable by timing, validity windows, pad class and public effects (section 3). | visible | visible | sees the timing | nullifier and commitment per transaction | `evidence/leak-surface.md` rows `nullifier`, `validity` | BUILT (accepted leaks listed in section 3) |
| 9 | Coordination content | not built (files today). Spike: chat-module groups are MLS end-to-end encrypted; transport metadata unmeasured | room members | the old room until rotation | not seen on LEZ | Logos chat module | `spikes/chat-module/run3/RESULTS.md` | PLANNED (M3) |
| 10 | Co-signing social graph | not on chain | room group membership | old room | not seen on LEZ | nothing is written on chain | design, ARCHITECTURE §4.2 | PLANNED (M3) |

**Public posture.** All ten items are public on chain except 5 and 9, because approvals still travel off-chain. The program is identical; only the account type changes. It is tested at handler level only. A sequencer e2e is PLANNED (B3).

## 2. What a private transaction publishes

The per-field table for a private `Execute`, with byte sizes, is in `evidence/leak-surface.md`. In summary:

- **Published:**
  - that a private transaction happened;
  - its nullifier, commitment and 4096-byte padded notes;
  - the fee amount and the fee inbox;
  - the validity window;
  - the transaction size.
- **Published by design when the action pays a public account:** the recipient and amount of the native credit. The sender shown is the native-token program, not the multisig.
- **Not published** (asserted by `private_tx_byte_scan`):
  - `V`;
  - member keys and account ids;
  - the fee payer id;
  - `P` and the image id (`Undisclosed` claim);
  - `refund_to`, which is asserted to be `None`.

The byte scan runs on dev-mode transactions. The real-proof benchmark runs assert a narrower set: no `P` or image id in the bytes, `refund_to = None`, note length, `Undisclosed` claims only. A full scan of a real-proof transaction is PLANNED (M1).

**Validity window published, per instruction:**

| Instruction | Window | Code |
|---|---|---|
| `Create` | none | `crates/msig-core/src/lib.rs:504-520` |
| `Execute` | `[.., expiry)` | `crates/msig-core/src/lib.rs:536` |
| `Queue` | `[old.expiry, t)` or `[.., t)` | `crates/msig-core/src/lib.rs:564` |
| `ExecuteQueued` | `[t + time_lock, expiry)` | `crates/msig-core/src/lib.rs:579` |
| `Spend` | `[p·period, (p+1)·period)` | `crates/msig-core/src/lib.rs:605` |

`Config` and `Cancel` are `Execute` or `ExecuteQueued` calls with another action.

## 3. Cross-layer correlations left observable (Pr.3)

1. **Timing.** Coordination traffic precedes a private transaction.
2. **Throughput.**
   - Storage gas allows at most 4 private transactions per block (`vendor/lez/lez/programs/fee/core/src/market.rs:27`, `:56`).
   - A 1 MiB block holds 3 transactions of 311,670 B. That narrows the candidates.
3. **Validity windows.**
   - The expiry, `t` and the spend period are public fingerprints. They reveal the time-lock and period lengths when transactions are linked by timing.
   - The CLI does not round them. It sets expiry to `now + --expiry-secs` (`crates/msig-cli/src/main.rs:428`). Rounding to a grid is PLANNED (M2).
4. **Pad class.**
   - Wallet patch 2 pads every note to 4096 bytes instead of the stock 512 (`vendor/PATCHES.md` §2).
   - Every transaction from this wallet is therefore distinguishable from stock-wallet transactions. The anonymity set is "transactions padded to 4096", which today means this application.
5. **Transaction size per operation.**
   - Real-proof sizes on image v2.1 (`evidence/bench/mac-cpu.log`, `docs/BENCHMARKS.md`):
     - `Create` 311,206 B;
     - `Execute` 311,670 B, at M = 5, 10 and 30;
     - fund 312,046 B.
   - So size reveals the operation type, but not M.
   - `Queue`, `ExecuteQueued`, `Spend` and `Config` sizes are not measured; that is PLANNED (M1).
6. **Fee amount** is public. `refund_to` must be `None`, and this is asserted (BUILT).
7. **Public effects** of an executed action: the recipient and amount.
8. **Note length** would leak if state on `V` outgrew the pad. At N = 30 the multisig state is about 1.2 KiB.
9. **The LEZ-to-chat binding** would exist only inside the end-to-end encrypted room. A room member or a compromised member device learns it. The room is not built: PLANNED (M3).
10. **Transport.** What the chat delivery layer sees (group id, sizes, IP addresses) is not specified by the chat docs and was not measured. Pr.2 is therefore not claimed. Measuring it is PLANNED (M3).
11. **Nullifier races.** A failed honest transaction followed by a retry is visible to the sequencer as a pattern.

## 4. Selective disclosure (RFP: configuration, holdings, activity)

- **Mechanism.** Give the audience the viewing secret key of `V`, never the GMS and never the nullifier secret.
  - BUILT: `viewing_key_alone_decrypts_state` opens a post-state note with the viewing key only.
  - PLANNED (M2): an export command. The wallet CLI has none.
- **Configuration and holdings.** Every state note decrypts, and it holds the members, threshold, roles, time lock, limit, epoch, nonce and balance. The audience can re-read the notes after later transactions to get current evidence.
- **Activity.**
  - The state keeps no action history.
  - A holder can decrypt the post-state note of each past transaction of `V`. That gives a sequence of nonce, epoch, balance and config changes. For each action, only its public effects are visible.
  - Only a single note is tested. History reconstruction is PLANNED (M2).
  - Which member approved what is not recorded on chain (item 4).
- **What any audience unavoidably learns.** The whole state, including every member's BIP-340 public key. There is no per-field or per-audience disclosure. A "wide audience" means the same key shared more widely.
- **Power granted.** Read-only. No spending power and no nullifier power, so no veto.
- **Revocation.** Only by re-keying into a new vault (PLANNED, M2). Any GMS holder can stall a re-key (section 5, item 1).

## 5. Known limits (read these)

1. **Any GMS holder can freeze the vault's funds. Reproduced, not fixed.**
   - Test `gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced`. A GMS holder lands a root message to `Actor(V, X)` for a trivial program X. It spends `V`'s nullifier, and an honest in-flight `Execute` built on the previous commitment is rejected.
   - Repeating this blocks every `Execute`, `Config`, `Queue`, `Spend` and the re-key for as long as the holder keeps doing it. That includes a removed member.
   - No funds move: spending authority is unaffected (`gms_holder_cannot_bypass_quorum`, cases (a), (a'), (b) and (d)).
   - There is no application-level fix. Mitigation path and the M1 decision gate: ARCHITECTURE §3.1.
2. **Removed member.**
   - Spending authority is revoked at once (`config_removes_member_and_stale_approvals_are_rejected`, e2e step 10).
   - Read access and the veto stay until a re-key.
3. **Pad class.** See section 3, item 4.
4. **Chat.**
   - No member removal.
   - A new address on every restart. A member who is offline misses messages for good (spike run 3).
   - A send that reports success is not delivery.
   - The key-package registry is a central point that observes who opens rooms.
   - Delivery-layer metadata is unmeasured.
5. **Validity windows are public** in every transaction.
6. **Creator trust.** The creator picks the GMS and can leak it. The creator is trusted for privacy and liveness, not for funds (ADVERSARY.md).
7. **Dev-mode limits.** Dev-mode proofs hide nothing about proof structure. The byte scan looks for known values (needles) only, not for receipt structure.
