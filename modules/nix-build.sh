#!/bin/sh
# `nix build "$@"` inside the container, retried: without a sandbox some builders create /homeless-shelter, which
# nix then refuses to build past; remove it and resume (finished derivations are kept).
n=0
until nix build "$@"; do
  rm -rf /homeless-shelter
  n=$((n + 1)); [ "$n" -gt 8 ] && exit 1
done
