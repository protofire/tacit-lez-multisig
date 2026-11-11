#!/bin/sh
# Builds the multisig guest with risc0's reproducible Docker builder (risczero/risc0-guest-builder
# r0.1.97.0 linux/amd64, pinned by digest in crates/msig-methods/build.rs) and prints its ImageID.
# Needs Docker; the result does not depend on the host OS/arch, the local toolchain or the checkout
# path. Separate target dir so it does not fight a concurrent non-Docker build for the cargo lock.
set -eu
cd "$(dirname "$0")/.."
RISC0_USE_DOCKER=1 CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/docker-host}" \
  cargo run -q --release --locked -p msig-methods --example image_id
