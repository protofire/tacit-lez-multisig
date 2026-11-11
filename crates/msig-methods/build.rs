use std::collections::HashMap;
use std::path::Path;

use risc0_build::{DockerOptionsBuilder, GuestOptionsBuilder};

/// risc0 reproducible guest builder, Rust 1.97.0, pinned to its linux/amd64 manifest digest
/// (the builder image published by RISC Zero).
const BUILDER_TAG: &str =
    "r0.1.97.0@sha256:6f6c300e4af57ea3efe62b7f04a17629efe0f0caa6ad3699d6766bf19e0e62fb";

fn main() {
    println!("cargo:rerun-if-env-changed=RISC0_USE_DOCKER");
    if std::env::var("RISC0_USE_DOCKER").as_deref() != Ok("1") {
        risc0_build::embed_methods();
        return;
    }
    // Docker build: context is the repo root (the guest has path deps into crates/ and vendor/).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docker = DockerOptionsBuilder::default()
        .root_dir(root)
        .docker_container_tag(BUILDER_TAG)
        .build()
        .unwrap();
    let opts = GuestOptionsBuilder::default().use_docker(docker).build().unwrap();
    risc0_build::embed_methods_with_options(HashMap::from([("msig-guest", opts)]));
}
