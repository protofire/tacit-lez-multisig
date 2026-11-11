#!/usr/bin/env bash
# Private-posture e2e on a fresh standalone LEZ v0.4.0-rc1 sequencer (dev mode, loopback only):
# deploy at P, create the private multisig, fund it, M-of-N Execute, sub-quorum Execute rejected,
# Config, Spend under a limit, Queue -> ExecuteQueued under a time lock.
# Transcript: evidence/e2e-private-transcript.txt. Dev keys only (derived from public labels).
#
# Env: MSIG_SEQ_WAIT (listener wait, s; default 60), MSIG_LEZ_PORT (default 3071; must be >= 3070), LEZ_SEQUENCER_BIN (sequencer_service built
# from LEZ 1806e87; required),
# MSIG_E2E_HOME (default target/e2e-private, wiped on start).
# RISC0_USE_DOCKER=1 deploys the frozen image v2.1 (evidence/IMAGE-ID-v2.1.md); without it the id embeds host paths.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${MSIG_LEZ_PORT:-3071}"
SEQ="${LEZ_SEQUENCER_BIN:?set LEZ_SEQUENCER_BIN to a sequencer_service built from LEZ 1806e87}"
H="${MSIG_E2E_HOME:-$ROOT/target/e2e-private}"
OUT="$ROOT/evidence/e2e-private-transcript.txt"
(( PORT >= 3070 )) || { echo "port $PORT: use 3070 or above" >&2; exit 2; }
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then echo "port $PORT is busy" >&2; exit 2; fi
[[ -x "$SEQ" ]] || { echo "no sequencer_service at $SEQ" >&2; exit 2; }

export PATH="$PATH:$HOME/.risc0/bin" RISC0_DEV_MODE=1
cd "$ROOT"
cargo build -q -p msig-host --bin msig-e2e
# reproducibility anchor for the transcript header: the guest image id of this build (same env as msig-e2e)
IMG="$(cargo run -q -p msig-methods --example image_id | sed -n 's/^msig //p')"
E2E="$ROOT/target/debug/msig-e2e"

rm -rf "$H"; mkdir -p "$H/wallet"
python3 -I - "$ROOT/vendor/lez/lez/sequencer/service/networks/local.json" "$H/seq-config.json" "$("$E2E" payer-pubkey)" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["block_create_timeout"] = "3s"
c["genesis"]["actions"].append({"supply_account": {"recipient": {"identified": {"key": sys.argv[3]}}, "balance": 10_000_000_000_000}})
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
python3 -I - "$ROOT/vendor/lez/lez/wallet/configs/debug/wallet_config.json" "$H/wallet/wallet_config.json" "$PORT" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["sequencer"]["sequencer_addr"] = f"http://127.0.0.1:{sys.argv[3]}"
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
"$SEQ" setup "$H/seqhome" --config "$H/seq-config.json" \
  --funding-public-key 2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26 --create-channel >"$H/seq-setup.log" 2>&1
"$SEQ" start "$H/seqhome" --rpc-listen-address "127.0.0.1:$PORT" --metrics-listen-address 127.0.0.1:0 >"$H/sequencer.log" 2>&1 &
SEQ_PID=$!
trap 'kill $SEQ_PID 2>/dev/null || true' EXIT
# the listener must be our sequencer: a foreign one on $PORT shares the dev keys and skews balances
# MSIG_SEQ_WAIT: seconds to wait for the listener (default 60; slower CI runners need more)
for _ in $(seq "${MSIG_SEQ_WAIT:-60}"); do
  [[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] && break
  kill -0 "$SEQ_PID" 2>/dev/null || break
  sleep 1
done
if [[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" != "$SEQ_PID" ]]; then
  kill -0 "$SEQ_PID" 2>/dev/null && state="still running" || state="exited"
  echo "sequencer $SEQ_PID not listening on $PORT ($state)" >&2
  echo "--- seq-setup.log (tail)" >&2; tail -n 40 "$H/seq-setup.log" >&2 || true
  echo "--- sequencer.log (tail)" >&2; tail -n 40 "$H/sequencer.log" >&2 || true
  exit 2
fi

{
  echo "# scripts/e2e-private.sh  $(date -u +%FT%TZ)  image msig $IMG"
  echo "# LEZ v0.4.0-rc1 1806e87 sequencer \$LEZ_SEQUENCER_BIN on 127.0.0.1:$PORT, RISC0_DEV_MODE=1 (proofs are dev-mode receipts)"
  LEE_WALLET_HOME_DIR="$H/wallet" "$E2E" run 2>&1 | grep --line-buffered -v -E '^\s*$|Scanning|^\[[0-9-]+T|^Transaction data is|^WARNING: [Pp]roving in dev mode|^(Latest block is|Syncing to block|Synced to block)'
} | tee "$OUT"
grep -q '^PASS$' "$OUT"
