# Leak surface of a private Execute

Source: `PrivacyPreservingTransaction { message: TransactionMessage<ProvenExecution>, witness_set }`
(vendor/lez `lee/state_machine/src/transaction_message.rs`, `core/src/circuit_io.rs`,
`core/src/execution_state.rs`). Enforced per field by `private_tx_byte_scan`
(crates/msig-host/tests/private.rs) for N = 3 and N = 30 (`MAX_MEMBERS`), tx with a private fee payer. Each field is
borsh-encoded and scanned for: V, fee payer id, P, the image id (LE words), and every member key
and member account id. Sizes are from the test run (identical for N = 3 and 30).

| Field | Bytes | Content | Verdict |
|---|---|---|---|
| message.public_context.actors | 132 | public recipient TO and the fee inbox only (asserted) | public by design; no V, P, member or payer id |
| message.public_context.authorized_accounts | 4 | empty | none |
| message.public_context.promotions | 4 | empty | none |
| message.nonces | 4 | empty (no public signer) | none |
| message.admission_evidence | 4 | empty | none |
| execution.boundary | 324 | native credit crossing to TO and the fee transfer to the inbox | public amounts/recipients by design; scanned clean |
| execution.async_messages | 4 | empty | none |
| execution.recovery_bindings | 4 | empty | none |
| execution.public_root | 1 | `None` (private root call; the `Actor(V, P)` target is not published) | none |
| execution.private_actions[i].nullifier | 32 | per-use nullifier | unlinkable hash; no needle |
| execution.private_actions[i].root | 32 | commitment-set root | public chain data |
| execution.private_actions[i].commitment | 32 | note commitment | hash; no needle |
| execution.private_actions[i].encrypted_post_state.epk | 1092 | ephemeral key (ML-KEM-sized) | random; no needle |
| execution.private_actions[i].encrypted_post_state.ciphertext | 4100 (4096 + 4 prefix) | padded note | length == NOTE_PAD for every note incl. the fee fallback note (asserted): hides N and config |
| execution.validity | 12 | block/timestamp window; `to = expiry` of the Execute | LEAK (accepted): the chosen expiry is public |
| execution.program_image_claims | 37 | exactly one `Undisclosed { root }` (asserted) | no P, no image id |
| execution.fee | 5455 | `ProvenFee { refund_to: None, fallback: Some(action, boundary) }` | refund_to None asserted; fallback note padded |
| witness_set | 16695 | no public signatures (first u32 == 0, asserted) + dev-mode proof | no signer keys |

Residual public information: the tx exists, TO and amount of the native credit, the fee inbox and
fee amount, the expiry, and tx size. Dev-mode proof bytes are not representative of a real receipt
(not scanned for structure beyond the needles).

## Undisclosed program claim

Before: `program_image_claims = [Disclosed { account_id: P, image_id }]`, which printed P and the
image id (the test failed with "P in execution.program_image_claims" when the Disclosed catalog was
put back). After: `ProgramCatalog::with_undisclosed(P, header, membership_proof)` with the
header read from the deployed immutable header account and the proof of
`immutable_mirror_commitment(P, &header)`; the claim carries a commitment-set root only.
