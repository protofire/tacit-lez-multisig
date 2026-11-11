//! multisig_module: Logos core module for the RFP-005 prototype (U.1): `create` and `execute`.
//! `core` is plain Rust (`cargo run --no-default-features --example direct`); the Logos glue is
//! behind the default `logos_module` feature.

pub mod core;

#[cfg(feature = "logos_module")]
mod glue;
