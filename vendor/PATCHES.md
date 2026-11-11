# Patches to vendored LEZ

`vendor/lez` is a `git archive` of logos-execution-zone `1806e87` (v0.4.0-rc1). The two changes below
are the only differences from that archive.

## 1. Wallet: shared private PDA bound to a deployed program account

File: `lez/wallet/src/lib.rs`.

`WalletCore::create_shared_pda_account` binds the PDA to `AccountId::from_builtin_program(image_id)`
(both in `for_private_pda` and in `PrivateAccountKind::Pda { account_id, .. }`). A program deployed
through the program loader lives at its header account `P`, and LEZ authority checks compare that
account (`lee/state_machine/core/src/execution_state.rs` `private_seed_grant`), so a PDA bound to
the builtin address can never receive the program's seed grant.

Change: new `create_shared_pda_account_at(group, seed, image_id, program_account_id)`, a copy of
the old body with `program_account_id` in both places; keys still come from
`GroupKeyHolder::derive_keys_for_pda(image_id, seed)`. The old function now calls it with
`from_builtin_program(image_id)`, so its behaviour is unchanged. It must live in the wallet crate
because `WalletCore::storage` is private.

## 2. Wallet: note pad 512 → 4096 (`NOTE_PAD`)

File: `lez/wallet/src/account_manager.rs`, `CIPHERTEXT_PAD_SIZE`.

The wallet pads every note (and its dummy notes) to this constant. 512 leaves 427 bytes for actor
states; the multisig state outgrows it from about N = 5 (358 B note at N = 3, 1,315 B at N = 32), so the note length would reveal N. Changed to 4096 =
`msig_core::NOTE_PAD` (ARCHITECTURE §3.2), below `MAX_CIPHERTEXT_PADDING` (8 KiB). One constant,
no new option. Side effect: every note this wallet emits is 4096 bytes, so its transactions are
distinguishable by size from stock-wallet (512) transactions; the anonymity set is "transactions
padded to 4096", not all private transactions.
