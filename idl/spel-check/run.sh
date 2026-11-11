#!/bin/sh
# Deserialises idl/msig.idl.json with SPEL's real SpelIdl structs (spel-framework-core/src/idl.rs,
# fetched at a pinned commit, serde only) and checks the program's instruction/error counts.
set -eu
cd "$(dirname "$0")"
SPEL_REV=c6f73ce
curl -fsSL "https://raw.githubusercontent.com/logos-co/spel/$SPEL_REV/spel-framework-core/src/idl.rs" -o src/idl.rs
cargo test --locked
