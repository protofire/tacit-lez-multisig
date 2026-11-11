# Coordination-room adversaries (RFP Coordination Architecture)

Base: `docs/ARCHITECTURE.md` §4. **The program is the only authority on validity**: the room carries no trust. A client signs only a digest it recomputed from bytes it decoded itself.

Status at image v2.1:
- **BUILT:** the on-chain checks (tests named) and the CLI's checks on envelope files (`docs/CLI.md`).
- **PLANNED (M3):** the room itself, `Bind`, `Invite` and chat provisioning.
- **Room evidence:** two spikes, both on preset `logos.dev`:
  - run 2: 2 members, placeholder payload (findings summarised in ARCHITECTURE §4.1; raw results not part of this repository);
  - `spikes/chat-module/run3/RESULTS.md`: 3 members, a 2-of-3 vault, a real proposal and approvals posted as `msig1:<kind>:` plus base64 of the CLI's JSON envelope, a forged approval dropped, then execution.

The "What the client does" column describes the CLI where it is built and the room client (PLANNED, M3) otherwise.

| Adversary action | Why it cannot decide validity | What the client does | Residual risk | Test |
|---|---|---|---|---|
| Withhold proposals or approvals | Approvals are self-authenticating BIP-340 signatures over a digest | Any member re-posts or carries them out of band; the CLI accepts approval files | Delay only. The chat module drops messages for offline members (run 3), so withholding and absence look the same | e2e builds approvals as files (`evidence/e2e-private-transcript.txt` steps 5-6) |
| Reorder | Validity depends on `(epoch, nonce)` in the digest, not arrival order | None needed | None | `replayed_approval_is_rejected` |
| Replay an old approval | Digest binds V, epoch, nonce, expiry, action hash; the nonce bumps on any execution | None needed | None | `replayed_approval_is_rejected`, `replayed_execute_is_rejected_after_nonce_bump`, e2e step 7 (BadSignature) |
| Inject a forged proposal or approval | In-guest signature check against current members | CLI: `show` marks a bad signature `INVALID`; `execute` counts only valid signatures of current members with the right permission (`crates/msig-cli/src/main.rs:677-691`). Flagging unbound senders is PLANNED (M3) | Wasted attention | `tampered_signature_is_rejected`, `non_member_key_under_member_index_is_rejected`; spike run 3 (forged approval dropped by the CLI; the program check was not exercised in that run) |
| Inject a valid proposal from a member without INITIATE | Proposer permission is checked in-guest | Same | None | `proposer_without_initiate_rejected` |
| Inject approvals from a member without VOTE | Only VOTE members count | Same | None | `proposer_without_vote_cannot_approve` |
| Split view (different proposals to different members) | Members may approve competing actions at one nonce; the first executed consumes the nonce, the rest die | Every approval names the full digest; listing all proposals per nonce is PLANNED (M3, B4) | Members can waste approvals; the chain decides | `cancel_consumes_nonce_and_kills_competing_proposals` |
| Sign both Approve and Reject | Reject is advisory; the chain only counts approvals | CLI reports the reject count and ignores it; a "likely dead" label is PLANNED (M3) | Label can be wrong | none |
| Withhold from the executor after M approvals | Any EXECUTE member holding M approvals may submit | Fall back to another executor | Delay | `non_executor_cannot_execute` |
| Any GMS holder races the nullifier | **Not tolerated by the design** | None possible at application level; mitigation path and M1 gate in ARCHITECTURE §3.1 | **Funds can be frozen**: honest `Execute`, `Config`, `Queue`, `Spend` and the re-key fail for as long as the holder repeats it; no funds move | `gms_holder_cannot_bypass_quorum_e_liveness_veto_is_reproduced` |
| Removed member | Approvals fail on chain at once | CLI counts only current members. Room: new room per epoch, member list rendered from chain state, flag removed senders (PLANNED, M3) | Reads the old room until rotation; keeps the veto above; can stall the re-key | `config_removes_member_and_stale_approvals_are_rejected`, e2e steps 9-10 |
| Malicious inviter or creator | A different GMS with the same `cfg, salt` yields a different, valid V with the same members | Compare V out of band before depositing (CLI `status` prints it); rejecting a `Bind` whose V differs is PLANNED (M3) | Creator trusted for privacy and liveness (can leak the GMS), not for funds | `create_with_wrong_keys_is_rejected`, `foreign_config_cannot_init` |
| Key-package registry outage | Not a validity matter | Opening a room fails; fall back to approval files | Cannot open or rejoin rooms while it is down. Spike: one transient failure of `devnet.chat-kc.logos.co` broke `create_conversation` | spike run 2 |
| "send ok" is not "delivered" | n/a | Never treat a send as an ack; confirm on chain (PLANNED, M3) | Two silent drops seen in the spike (message over 153600 bytes; RLN missing on `logos.test`), both with `success:true` | spike run 2 |
| Oversize or RLN-blocked sends | n/a | Keep approval payloads below the limit (about 112 KB base64) and use preset `logos.dev` | `logos.test` needs an RLN module that the catalog does not install | spike run 2 |

Tests are in `crates/msig-core/tests/handler.rs` and `crates/msig-host/tests/private.rs`.

## Not covered

- **Room membership reconciliation on a config change.** The design is a new room per epoch, because the chat module cannot remove members. Not built: PLANNED (M3).
- **Offline members.** Messages posted while a member is offline never reach them. After a restart the member has a new address, and the old group is `history_only` (spike run 3, step 6). The design is re-bind, re-add and re-post of pending envelopes. Not built: PLANNED (M3).
- **Transport metadata** (IP addresses, group ids): unmeasured, PLANNED (M3).
- **Ordering:** not tested. A non-member's valid signature in `show`: not tested (spike run 3).
- **Network:** only `logos.dev` was used. Whether the chat module runs on the network that pairs with testnet 0.4 is open.
