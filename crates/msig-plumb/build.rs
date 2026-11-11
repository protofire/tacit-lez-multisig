// Plain (non-Docker) build: the plumbing image is a test-only measurement tool with its own id,
// never deployed, so it does not need the reproducible builder.
fn main() {
    risc0_build::embed_methods();
}
