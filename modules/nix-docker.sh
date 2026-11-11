#!/usr/bin/env bash
# Run a command with Nix inside Docker, never installing Nix on the host.
# /nix = overlayfs: lower = read-only volume $NIX_LOWER_VOLUME (default tacit-nix: a Nix install, seeded once from the
# nixos/nix image if the volume does not exist; never written afterwards), upper = volume $NIX_UPPER_VOLUME
# (default tacit-nix-upper: the build cache). Needs --privileged only for the overlay mount.
# The repo is mounted at its host path (the git+file flake and vendor/ need it); the container reaches a sequencer on
# the host as host.docker.internal.
#   modules/nix-docker.sh build      nix build the module .lgx -> /tmp/lgx (logs)
#   modules/nix-docker.sh headless   logosctl run of create + execute (needs the host sequencer: modules/headless/headless.sh)
#   modules/nix-docker.sh -- <cmd>   any command
# New files must be `git add`-ed before building: the flake is a git+file flake and ignores untracked files.
set -euo pipefail
ROOT="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
GIT_COMMON="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)"
FLAKE="git+file://$ROOT?dir=modules/multisig_module"
LOWER="${NIX_LOWER_VOLUME:-tacit-nix}"; UPPER="${NIX_UPPER_VOLUME:-tacit-nix-upper}"
# an empty named volume mounted on /nix is populated from the image's /nix (Nix, its profile and CA bundle)
docker volume inspect "$LOWER" >/dev/null 2>&1 || docker run --rm -v "$LOWER:/nix" nixos/nix:latest true
case "${1:-build}" in
  build)    CMD="'$ROOT/modules/nix-build.sh' -L '$FLAKE#lgx' --out-link /tmp/lgx && ls -la /tmp/lgx/" ;;
  headless) CMD="bash '$ROOT/modules/headless/run.sh'" ;;
  --)       shift; CMD="$*" ;;
  *)        echo "usage: $0 build|headless|-- cmd" >&2; exit 2 ;;
esac
exec docker run --rm --privileged -v "$LOWER:/lower:ro" -v "$UPPER:/upper" \
  -v "$ROOT:$ROOT" -v "$GIT_COMMON:$GIT_COMMON" \
  -e ROOT="$ROOT" -e FLAKE="$FLAKE" -e WALLET_HOME="${WALLET_HOME:-}" -e MSIG_MEMBERS="${MSIG_MEMBERS:-3}" -e MSIG_THRESHOLD="${MSIG_THRESHOLD:-2}" \
  ${DOCKER_ARGS:-} \
  -e NIX_CONFIG='experimental-features = nix-command flakes
accept-flake-config = true
sandbox = false
build-users-group =
max-jobs = auto
cores = 0
substituters = https://cache.nixos.org https://cache.nix.logos.co/public
trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= public:l4HrXgL4nw246+LBh2SOJyhz64BoGegOYLheT/iIAPU=' \
  -e SSL_CERT_FILE=/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt \
  -e GIT_SSL_CAINFO=/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt \
  -e NIX_SSL_CERT_FILE=/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt \
  -w "$ROOT" alpine:latest sh -c "mkdir -p /upper/u /upper/w /nix && mount -t overlay overlay -o lowerdir=/lower,upperdir=/upper/u,workdir=/upper/w /nix && mkdir -p /nix/var/log/nix/drvs /nix/home && export PATH=/nix/var/nix/profiles/default/bin:\$PATH HOME=/nix/home USER=root && git config --global --add safe.directory '*'; $CMD"
