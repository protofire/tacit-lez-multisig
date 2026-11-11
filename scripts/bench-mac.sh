#!/usr/bin/env bash
# P.1 on a MacBook (owner runs it, machine otherwise idle): real proofs of a private Execute at
# M=5 (N=7) and M=10 (N=12), frozen image v2.1, against a local standalone sequencer (port >= 3075).
# Two variants: cpu (risc0 prove, no GPU) and metal (risc0 `metal` feature). Per variant and M:
# 1 warm-up + 3 runs, each run its own process under `/usr/bin/time -l` (peak RSS).
# Logs: evidence/bench/mac-<variant>.log (raw) and a `SUMMARY` line per run in the same file.
#
#   RISC0_USE_DOCKER=1 LEZ_SEQUENCER_BIN=/path/sequencer_service scripts/bench-mac.sh [cpu|metal|both]
# Env: BENCH_PORT (default 3075). Takes roughly 15-40 min per variant; GPU/CPU usage is total.
set -euo pipefail
case "${1:-}" in -h|--help) sed -n '2,11p' "$0"; exit 0;; esac
VARIANTS="${1:-cpu}"; [[ "$VARIANTS" == both ]] && VARIANTS="cpu metal"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
[[ "$(uname -s)" == Darwin ]] || { echo "macOS only" >&2; exit 2; }
[[ "${RISC0_USE_DOCKER:-}" == 1 ]] || { echo "set RISC0_USE_DOCKER=1 (frozen image v2.1)" >&2; exit 2; }
SEQ="${LEZ_SEQUENCER_BIN:?path to sequencer_service (LEZ 1806e87)}"; [[ -x "$SEQ" ]] || { echo "no $SEQ" >&2; exit 2; }
PORT="${BENCH_PORT:-3075}"; (( PORT >= 3075 )) || { echo "port: 3075 or above" >&2; exit 2; }
busy="$(ps -axo pid=,comm=,args= | grep -E '(^| )(cargo|rustc|sequencer_service|msig-bench|r0vm)( |$)|docker[^ ]* +(buildx? )?build|docker.*risc0-guest-builder' | grep -v -E 'grep|bench-mac' || true)"
[[ -z "$busy" ]] || { echo "refusing to start: heavy processes running:" >&2; echo "$busy" >&2; exit 2; }
lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1 && { echo "port $PORT busy" >&2; exit 2; }
OUT="$ROOT/evidence/bench"; mkdir -p "$OUT"
MODEL="$(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.model), $(( $(sysctl -n hw.memsize) / 1073741824 )) GB, $(sw_vers -productName) $(sw_vers -productVersion)"
unset RISC0_DEV_MODE; export BENCH_MODE=real PATH="$PATH:$HOME/.risc0/bin"
for V in $VARIANTS; do
  LOG="$OUT/mac-$V.log"; : >"$LOG"
  FEAT="e2e,prove"; [[ "$V" == metal ]] && FEAT="e2e,metal"
  { echo "# $(date -u +%FT%TZ) variant=$V  image msig v2.1 (asserted per run)  LEZ v0.4.0-rc1 1806e87"; echo "# machine: $MODEL"
    echo "# cores: $(sysctl -n hw.perflevel0.physicalcpu)P+$(sysctl -n hw.perflevel1.physicalcpu)E; uptime: $(uptime | sed 's/.*up/up/')"
    echo "# $(sysctl vm.swapusage); $(memory_pressure | tail -1)"
    echo "# rustc $(rustc --version); features $FEAT; risc0-zkvm 3.0.5"; } | tee -a "$LOG"
  # Host-only build fix (guest image unchanged): the macOS 27 Metal compiler rejects `cells[i] = 0` on Fp in
  # risc0-sys 1.5.0 poseidon2.metal ("ambiguous operator="); build against a copy with `= Fp(0)`.
  RS="$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/risc0-sys-1.5.0 | head -1)"; FIX="$ROOT/target/risc0-sys-metal-fix"
  rm -rf "$FIX"; cp -R "$RS" "$FIX"; sed -i '' 's/\] = 0;/] = Fp(0);/' "$FIX/kernels/zkp/metal/poseidon2.metal"
  PATCH=(--config "patch.crates-io.risc0-sys.path=\"$FIX\"")
  # BENCH_PREBUILT=1: binaries already built (lets Docker be stopped during the run; msig-bench asserts the image id)
  [[ "${BENCH_PREBUILT:-}" == 1 ]] || cargo build -q --release "${PATCH[@]}" -p msig-host --no-default-features --features "$FEAT" --bins
  git checkout -q -- Cargo.lock  # the --config patch only touches the lock file's source line
  TD="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; B="$TD/release/msig-bench"
  H="$ROOT/target/bench-mac-$V"; rm -rf "$H"; mkdir -p "$H/wallet"; export LEE_WALLET_HOME_DIR="$H/wallet"
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
  SEQ_PID=$!; trap 'kill $SEQ_PID 2>/dev/null || true' EXIT
  for _ in $(seq 60); do [[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] && break; sleep 1; done
  [[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] || { echo "sequencer not listening on $PORT" >&2; exit 2; }
  step() { # tag args...: one process under time -l; echo RESULT + a SUMMARY line
    local tag="$1"; shift; local t; t="$(mktemp)"
    /usr/bin/time -l "$B" "$@" >"$t.out" 2>"$t.err" & local tp=$!
    # core diagnostic: system-wide CPU% sampled every 2 s while the step runs (100% = one core busy)
    while kill -0 "$tp" 2>/dev/null; do ps -A -o %cpu= | awk '{s+=$1} END {printf "%d\n", s}' >>"$t.cpu"; sleep 2; done
    wait "$tp" || { cat "$t.out" "$t.err" >&2; return 1; }
    local rss; rss=$(awk '/maximum resident set size/ {printf "%d", $1/1048576}' "$t.err")
    local cores; cores=$(awk '/ real / {printf "cpu_user_s=%.0f cpu_sys_s=%.0f wall_s=%.0f", $3, $5, $1}' "$t.err")
    cores="$cores $(sort -n "$t.cpu" | awk '{a[NR]=$1; s+=$1} END {printf "sys_cpu_pct_avg=%d sys_cpu_pct_p90=%d sys_cpu_pct_max=%d", s/NR, a[int(NR*0.9)+(NR*0.9>int(NR*0.9))], a[NR]}')"
    grep -E '^(RESULT|CHECK)' "$t.out" | sed "s/^/[$tag] /" | tee -a "$LOG"
    echo "SUMMARY variant=$V tag=$tag max_rss_mib=$rss $cores $(grep -o 'send_tx_s=[^ ]*\|prove_est_s=[^ ]*\|e2e_client_s=[^ ]*\|tx_bytes=[^ ]*\|proof_bytes=[^ ]*\|block=[^ ]*' "$t.out" | tr '\n' ' ')" | tee -a "$LOG"
    rm -f "$t" "$t.out" "$t.err" "$t.cpu"
  }
  step init init
  for MN in "5 7" "10 12"; do
    set -- $MN; M=$1; N=$2
    step "create-m$M" create "$M" "$N" "c$M"; step "fund-m$M" fund "c$M" 100000
    for i in 0 1 2 3; do step "exec-m$M-$([[ $i == 0 ]] && echo warmup || echo run$i)" exec "c$M"; done
  done
  kill "$SEQ_PID" 2>/dev/null || true; wait "$SEQ_PID" 2>/dev/null || true; trap - EXIT
done
