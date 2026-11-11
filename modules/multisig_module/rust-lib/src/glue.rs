//! Logos module glue for `multisig_module` (rust-first authoring). Results cross as JSON strings:
//! `{ "ok": true, ... }` / `{ "ok": false, "error" }`.

use crate::core;

pub trait MultisigModule: Send + 'static {
    /// Deploys the multisig program at P if absent, funds the fee payer, creates a private
    /// `threshold`-of-`members` multisig V (dev members) and credits it `fund` native units.
    /// `wallet_home` = directory with `wallet_config.json` (sequencer address). Slow: each private
    /// transaction is simulated, proven and awaited. `{ ok, vault, block, balance, ... }`.
    fn create(&mut self, wallet_home: String, members: i64, threshold: i64, fund: String) -> String;
    /// Executes a quorum-approved native transfer of `amount` from V to the dev account
    /// `recipient_label`. `signers` = comma-separated dev member indices (e.g. "1,2"); member 0
    /// always proposes and executes. `{ ok, block, vaultBalance, recipientBalance, ... }`.
    fn execute(&mut self, wallet_home: String, recipient_label: String, amount: String, signers: String) -> String;

    fn on_context_ready(&mut self, _ctx: &RustModuleContext) {}
}

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

#[derive(Default)]
struct Multisig;

impl MultisigModule for Multisig {
    fn create(&mut self, wallet_home: String, members: i64, threshold: i64, fund: String) -> String {
        core::create(&wallet_home, members, threshold, &fund).to_string()
    }

    fn execute(&mut self, wallet_home: String, recipient_label: String, amount: String, signers: String) -> String {
        core::execute(&wallet_home, &recipient_label, &amount, &signers).to_string()
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<Multisig>();
}
