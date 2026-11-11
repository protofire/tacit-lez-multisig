//! Test-only program `X` for `gms_holder_cannot_bypass_quorum` (e): accepts any message and keeps
//! its state. Never deployed outside the in-process harness.
fn main() {
    lee_core::program::run_actor_with(|_| lee_core::program::Response::keep_state())
}
