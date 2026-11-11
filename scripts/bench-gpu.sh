#!/usr/bin/env bash
# P.1/P.2 benchmark on the GPU box (Linux, CUDA), frozen image v2.1, against a standalone LEZ v0.4
# sequencer on a private port (>= 3075). Modes:
#   real  RISC0_DEV_MODE unset: Create/fund/Execute at M=5 (N=7), M=10 (N=12), M=30 (N=30, MAX_MEMBERS), 1 warm-up
#         + 3 runs for Execute at M=5/10 (1 warm-up + 1 run at M=30; Create and fund: warm-up + 1)
#   dev   negative control: RISC0_DEV_MODE=1 on a dev-mode sequencer, one Execute at M=5, NOT a benchmark
#   cycles  P.2: executor cycles/segments of msig vs the signature-free plumbing guest
# Needs RISC0_USE_DOCKER=1 (the binaries assert image v2.1 at runtime), LEZ_SEQUENCER_BIN (built from LEZ 1806e87).
# Logs: evidence/bench/<mode>-*.log
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
MODE="${1:?real|dev|cycles}"
export PATH="/usr/local/cuda/bin:$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH"
[[ "${RISC0_USE_DOCKER:-}" == 1 ]] || { echo "set RISC0_USE_DOCKER=1" >&2; exit 2; }
OUT="$ROOT/evidence/bench"; mkdir -p "$OUT"
TD="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
unset RISC0_DEV_MODE
{
  echo "# $(date -u +%FT%TZ) $MODE  image msig v2.1 (asserted per run)  LEZ v0.4.0-rc1 1806e87"
  echo "# $(uname -srm); $(nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader); $(nvcc --version | tail -1)"
  echo "# rustc $(rustc --version); risc0-zkvm 3.0.5"
} | tee "$OUT/$MODE-env.log"
cargo build -q --release -p msig-host --no-default-features --features e2e,cuda --bins
if [[ "$MODE" == cycles ]]; then
  "$TD/release/msig-cycles" 1 2 5 10 16 24 30 | tee "$OUT/cycles.log"; exit 0
fi

PORT="${BENCH_PORT:-3075}"; (( PORT >= 3075 )) || { echo "port: 3075 or above" >&2; exit 2; }
SEQ="${LEZ_SEQUENCER_BIN:?path to sequencer_service}"
if [[ "$MODE" == dev ]]; then export RISC0_DEV_MODE=1 BENCH_MODE=dev; else export BENCH_MODE=real; fi
H="${BENCH_HOME:-$ROOT/target/bench-$MODE}"; rm -rf "$H"; mkdir -p "$H/wallet"
export LEE_WALLET_HOME_DIR="$H/wallet"
B="$TD/release/msig-bench"
python3 -I - "$ROOT/vendor/lez/lez/sequencer/service/networks/local.json" "$H/seq-config.json" "$("$B" payer-pubkey)" <<'PY'
import json, sys
c = json.load(open(sys.argv[1])); c["block_create_timeout"] = "3s"
c["genesis"]["actions"].append({"supply_account": {"recipient": {"identified": {"key": sys.argv[3]}}, "balance": 10_000_000_000_000}})
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
python3 -I - "$ROOT/vendor/lez/lez/wallet/configs/debug/wallet_config.json" "$H/wallet/wallet_config.json" "$PORT" <<'PY'
import json, sys
c = json.load(open(sys.argv[1])); c["sequencer"]["sequencer_addr"] = f"http://127.0.0.1:{sys.argv[3]}"
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
"$SEQ" setup "$H/seqhome" --config "$H/seq-config.json" \
  --funding-public-key 2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26 --create-channel >"$H/seq-setup.log" 2>&1
"$SEQ" start "$H/seqhome" --rpc-listen-address "127.0.0.1:$PORT" --metrics-listen-address 127.0.0.1:0 >"$H/sequencer.log" 2>&1 &
SEQ_PID=$!; echo "$SEQ_PID" >"$H/sequencer.pid"
trap 'kill $SEQ_PID 2>/dev/null || true' EXIT
for _ in $(seq 60); do [[ "$(ss -ltnp 2>/dev/null | grep ":$PORT " | grep -c "pid=$SEQ_PID,")" == 1 ]] && break; sleep 1; done
[[ "$(ss -ltnp | grep ":$PORT " | grep -c "pid=$SEQ_PID,")" == 1 ]] || { echo "sequencer $SEQ_PID not listening on $PORT" >&2; exit 2; }
echo "# sequencer $SEQ pid $SEQ_PID port $PORT RISC0_DEV_MODE=${RISC0_DEV_MODE:-unset}" | tee -a "$OUT/$MODE-env.log"

LOG="$OUT/$MODE-run.log"; : >"$LOG"
step() { local tag="$1"; shift; echo "## $tag: $*" | tee -a "$LOG"; scripts/real-proof.sh "$@" 2>&1 | grep --line-buffered -E '^(RESULT|STAT|CHECK)|rror|panick' | sed "s/^/[$tag] /" | tee -a "$LOG"; }
runs() { local label="$1" n="$2" i; for ((i = 0; i < n; i++)); do step "exec-$label-$([[ $i == 0 ]] && echo warmup || echo run$i)" exec "$label"; done; }

step init init
if [[ "$MODE" == dev ]]; then
  step create-c5 create 5 7 c5; step fund-c5 fund c5 100000; step exec-dev exec c5; exit 0
fi
step create-warmup create 5 7 warm; step fund-warmup fund warm 1000
step create-c5 create 5 7 c5;       step fund-c5 fund c5 100000;   runs c5 4
step create-c10 create 10 12 c10;   step fund-c10 fund c10 100000; runs c10 4
step create-c30 create 30 30 c30;   step fund-c30 fund c30 100000; runs c30 2
echo "DONE" | tee -a "$LOG"
