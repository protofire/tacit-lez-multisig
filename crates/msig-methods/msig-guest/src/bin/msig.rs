use risc0_zkvm::guest::env;

fn main() {
    lee_core::program::run_actor_with(|input| {
        msig_core::handle(input).unwrap_or_else(|e| env::exit(e as u8))
    })
}
