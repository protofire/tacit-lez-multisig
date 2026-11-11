#!/usr/bin/env bash
# Reproduce: starts a fresh standalone LEZ v0.4.0-rc1 sequencer on the host (dev mode, port >= 3072), prepares a wallet
# home, then runs `create` and `execute` through the Logos headless host (nix-in-docker, MODE=logos, default) or
# through the module's Rust lib directly (MODE=direct, the fallback). Transcript: evidence/module-headless-transcript.txt.
# Env: MSIG_LEZ_PORT (default 3072), LEZ_SEQUENCER_BIN (sequencer_service from LEZ 1806e87, read-only use), MODE
# (logos, direct, or retry: the retry-safety scenarios H1/H2, OUT=... to keep the transcript).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PORT="${MSIG_LEZ_PORT:-3072}"
MODE="${MODE:-logos}"
SEQ="${LEZ_SEQUENCER_BIN:?set LEZ_SEQUENCER_BIN to a sequencer_service built from LEZ 1806e87}"
H="$ROOT/target/module-headless"
OUT="${OUT:-$ROOT/evidence/module-headless-transcript.txt}"
(( PORT >= 3072 )) || { echo "port $PORT: use 3072 or above" >&2; exit 2; }
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then echo "port $PORT is busy" >&2; exit 2; fi
[[ -x "$SEQ" ]] || { echo "no sequencer_service at $SEQ" >&2; exit 2; }

export PATH="$PATH:$HOME/.risc0/bin" RISC0_DEV_MODE=1
cd "$ROOT"
cargo build -q -p msig-host --bin msig-e2e
# host builds of rust-lib need the optional logos-rust-sdk path to exist; the Logos builder supplies the real SDK
S=modules/multisig_module/logos-rust-sdk-src
[[ -f $S/Cargo.toml ]] || { mkdir -p $S/src; printf '[package]\nname = "logos-rust-sdk"\nversion = "0.0.0"\nedition = "2021"\n' >$S/Cargo.toml; : >$S/src/lib.rs; }
rm -rf "$H"; mkdir -p "$H/wallet"
# the container reaches the host's loopback sequencer as host.docker.internal
ADDR="127.0.0.1"; [[ $MODE == logos ]] && ADDR="host.docker.internal"
python3 -I - "$ROOT/vendor/lez/lez/sequencer/service/networks/local.json" "$H/seq-config.json" "$(target/debug/msig-e2e payer-pubkey)" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["block_create_timeout"] = "3s"
c["genesis"]["actions"].append({"supply_account": {"recipient": {"identified": {"key": sys.argv[3]}}, "balance": 10_000_000_000_000}})
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
python3 -I - "$ROOT/vendor/lez/lez/wallet/configs/debug/wallet_config.json" "$H/wallet/wallet_config.json" "http://$ADDR:$PORT" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["sequencer"]["sequencer_addr"] = sys.argv[3]
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
"$SEQ" setup "$H/seqhome" --config "$H/seq-config.json" \
  --funding-public-key 2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26 --create-channel >"$H/seq-setup.log" 2>&1
"$SEQ" start "$H/seqhome" --rpc-listen-address "127.0.0.1:$PORT" --metrics-listen-address 127.0.0.1:0 >"$H/sequencer.log" 2>&1 &
SEQ_PID=$!
trap 'kill $SEQ_PID 2>/dev/null || true' EXIT
for _ in $(seq 60); do lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1 && break; sleep 1; done

{
  echo "# modules/headless/headless.sh MODE=$MODE  $(date -u +%FT%TZ)"
  echo "# LEZ v0.4.0-rc1 1806e87 sequencer \$LEZ_SEQUENCER_BIN on 127.0.0.1:$PORT, RISC0_DEV_MODE=1 (proofs are dev-mode receipts)"
  if [[ $MODE == logos ]]; then
    WALLET_HOME="$H/wallet" modules/nix-docker.sh headless
  elif [[ $MODE == retry ]]; then
    # Retry-safety scenarios (H1, H2) through the Rust lib. Each step prints the JSON result.
    cd modules/multisig_module/rust-lib
    cargo build -q --no-default-features --example direct
    D() { echo; echo "\$ direct $*"; target/debug/examples/direct "$@" || true; }
    W="$H/wallet"; CFG="$W/wallet_config.json"
    polls() { python3 -I -c 'import json,sys; c=json.load(open(sys.argv[1])); c["seq_tx_poll_max_blocks"]=int(sys.argv[2]); json.dump(c,open(sys.argv[1],"w"),indent=1)' "$CFG" "$1"; }
    D create "$W" 3 2 1000
    D execute "$W" alice 100 1,2
    echo; echo "# H2: inclusion polling fails after submit (seq_tx_poll_max_blocks=1), then the caller retries the same payment"
    polls 1; D execute "$W" alice 300 1,2
    sleep 8; polls 15
    D execute "$W" alice 300 1,2
    echo "# H2 expect: alice ends at 401 (1 + 100 + 300), V at 600; a second 300 is a double payment"
    echo; echo "# H1: create died after funding, before the marker (simulated: msig.json removed), then the caller retries create"
    mv "$W/msig.json" "$W/msig.json.v1"
    D create "$W" 3 2 1000
    echo "# H1 expect: refused (the group of V1 must survive); then V1 still executes"
    mv "$W/msig.json.v1" "$W/msig.json"
    D execute "$W" alice 50 1,2
  else
    echo "# FALLBACK: module core called directly through its Rust lib, NOT through the Logos host"
    cd modules/multisig_module/rust-lib
    cargo build -q --no-default-features --example direct
    D=target/debug/examples/direct
    echo; echo "\$ direct create $H/wallet 3 2 1000";  "$D" create  "$H/wallet" 3 2 1000
    echo; echo "\$ direct execute $H/wallet alice 300 1,2"; "$D" execute "$H/wallet" alice 300 1,2
  fi
} 2>&1 | grep --line-buffered -v -E '^\s*$|Scanning|^\[[0-9-]+T|^Transaction data is|^WARNING: [Pp]roving in dev mode|^(Latest block is|Syncing to block|Synced to block)' | tee "$OUT"
# first line: what ran (guest image id from the module's pinned MSIG_ID words, program address P from the run)
python3 -I - "$OUT" modules/multisig_module/rust-lib/crates/msig-methods/src/lib.rs <<'PY'
import re, struct, sys
out, src = sys.argv[1:]
t = open(out).read()
words = [int(x) for x in re.search(r"MSIG_ID: \[u32; 8\] = \[([^\]]*)\]", open(src).read()).group(1).split(",")]
p = re.search(r'program\\?":\\?"(\w+)', t)
open(out, "w").write(f"# image msig {struct.pack('<8I', *words).hex()}  program {p.group(1) if p else 'NONE (no create result)'}\n" + t)
PY
