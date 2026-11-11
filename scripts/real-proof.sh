#!/usr/bin/env bash
# One measured msig-bench operation (one process): wall time, peak RSS (/usr/bin/time) and peak GPU
# memory (nvidia-smi sampled every 0.5 s, minus the idle baseline). Runs the binary from the
# RESOLVED target dir (CARGO_TARGET_DIR / .cargo config), never a stale ./target path.
# Usage: scripts/real-proof.sh <msig-bench args...>   (env as bench-gpu.sh: LEE_WALLET_HOME_DIR, BENCH_MODE)
# Prints the bench output, then `STAT wall_s=.. max_rss_mib=.. gpu_before_mib=.. gpu_peak_mib=.. gpu_peak_delta_mib=..`.
set -euo pipefail
cd "$(dirname "$0")/.."
TD="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
BIN="$TD/release/msig-bench"
[[ -x "$BIN" ]] || { echo "no $BIN: build it first (scripts/bench-gpu.sh does)" >&2; exit 2; }
[[ "${BENCH_MODE:-}" == dev ]] || unset RISC0_DEV_MODE
export PATH="/usr/local/cuda/bin:$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH"
gpu() { nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null || echo 0; }
peak="$(mktemp)"; tm="$(mktemp)"; before="$(gpu)"; echo "$before" >"$peak"
( p=$before; while :; do m=$(gpu); if (( m > p )); then p=$m; echo "$p" >"$peak"; fi; sleep 0.5; done ) & S=$!
trap 'kill $S 2>/dev/null || true; rm -f "$peak" "$tm"' EXIT
/usr/bin/time -f '%e %M' -o "$tm" "$BIN" "$@"
read -r wall rss <"$tm"; pk="$(cat "$peak")"
echo "STAT wall_s=$wall max_rss_mib=$((rss / 1024)) gpu_before_mib=$before gpu_peak_mib=$pk gpu_peak_delta_mib=$((pk - before))"
