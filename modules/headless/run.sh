#!/usr/bin/env bash
# Headless Logos Core run (RFP U.3): logosctl daemon + multisig_module, then `create` and `execute` against the
# sequencer named in $WALLET_HOME/wallet_config.json. Runs INSIDE the nix container (modules/nix-docker.sh headless),
# started by modules/headless/headless.sh, which prepares the sequencer and the wallet home.
# multisig_module declares no module dependencies, so there is nothing else to install. (A module that names
# dependencies does not load unless every one is built and installed here.)
set -euo pipefail
W="$(mktemp -d)"
run() { echo; echo "\$ ${*/#$W\//}"; "$@" 2>&1 || { echo "(exit $?)"; return 1; }; }

nix build 'github:logos-co/logos-logoscore-cli#ctl' --out-link "$W/logos"
"$ROOT/modules/nix-build.sh" -L "$FLAKE#lgx" --out-link "$W/msig-lgx" >&2

export RISC0_DEV_MODE=1                          # proofs are dev-mode receipts, as the sequencer expects
export LOGOSCTL_TIMEOUT="${LOGOSCTL_TIMEOUT:-900000}"   # each private tx is simulated, proven and awaited
export HOME="$W/home"; mkdir -p "$HOME"
ctl() { "$W/logos/bin/logosctl" --config-dir "$W/session" "$@"; }
trap 'ctl daemon stop >/dev/null 2>&1 || true' EXIT
echo "# multisig_module headless transcript — $(date -u +%FT%TZ), $(uname -m)-linux, RISC0_DEV_MODE=1"
echo "# sequencer: $(grep -o '"sequencer_addr": *"[^"]*"' "$WALLET_HOME/wallet_config.json")"

run ctl daemon start --detach
run ctl install "$W"/msig-lgx/*.lgx -y
run ctl package ls
run ctl module load multisig_module
# `ctl call` succeeds whenever the RPC does; the module's verdict is the inner result JSON (escaped in "result")
call() { local o; o="$(run ctl call multisig_module "$@")" || { echo "$o"; return 1; }; echo "$o"
         grep -qF '\"ok\":true' <<<"$o" || { echo "(inner result not ok)"; return 1; }; }
call create "$WALLET_HOME" "$MSIG_MEMBERS" "$MSIG_THRESHOLD" str:1000
call execute "$WALLET_HOME" alice str:300 str:1,2
