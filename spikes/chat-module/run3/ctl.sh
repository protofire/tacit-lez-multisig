#!/bin/sh
# usage: ctl.sh A|B|C <logosctl args> -- one daemon, session dir and HOME per member (native macOS, released logosctl 0.3.2)
# LOGOSCTL: path to the logosctl binary from the release asset logosctl-aarch64-macos.tar.gz (logos-co/logos-logoscore-cli 0.3.2)
R=$(cd "$(dirname "$0")" && pwd); w=$1; shift
: "${LOGOSCTL:?set LOGOSCTL to the logosctl 0.3.2 binary}"
export HOME=$R/home/$w LOGOSCTL_TIMEOUT=${LOGOSCTL_TIMEOUT:-120000}; mkdir -p "$HOME"
exec "$LOGOSCTL" --config-dir "$HOME/session" "$@"
