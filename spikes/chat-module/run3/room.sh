#!/usr/bin/env bash
# run3: a 2-of-3 msig coordination room on the real Logos chat module (logos.dev), three members A B C,
# each with its own logosctl daemon + msig wallet home. A proposes into the room, B and C decode,
# show, approve and post back; B also posts a forged approval; A validates each approval with
# `msig show`, merges, executes on a standalone dev-mode LEZ sequencer (:3085). Round 2: C offline
# while B approves, then C comes back (async check).
# Dev-mode proofs (RISC0_DEV_MODE=1). Transcripts: a.log b.log c.log (per member), room.log (timings).
# Env: LEZ_SEQUENCER_BIN, LOGOSCTL, LGX_DIR (see RESULTS.md "Prerequisites").
set -uo pipefail
R="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$R/../../.." && pwd)"
PORT=3085
SEQ="${LEZ_SEQUENCER_BIN:?set LEZ_SEQUENCER_BIN to a sequencer_service built from LEZ 1806e87}"
MSIG="$ROOT/target/debug/msig"
LGX="${LGX_DIR:?set LGX_DIR to a dir holding the release chat_module-0.3.0.lgx and delivery_module-0.3.0.lgx}"
: "${LOGOSCTL:?set LOGOSCTL to the logosctl 0.3.2 binary (see ctl.sh)}"
export RISC0_DEV_MODE=1
[[ -x "$MSIG" ]] || { echo "build first: cargo build -p msig-cli" >&2; exit 2; }
lsof -nP -iTCP:$PORT -sTCP:LISTEN >/dev/null 2>&1 && { echo "port $PORT busy" >&2; exit 2; }
rm -rf "$R/home" "$R/out" "$R/w" "$R/net"; mkdir -p "$R/out" "$R/net" "$R/w"/{A,B,C}
: > "$R/a.log"; : > "$R/b.log"; : > "$R/c.log"; : > "$R/room.log"

ts() { python3 -c 'import time;print(f"{time.time():.1f}")'; }
lc() { echo "$1" | tr ABC abc; }
log() { echo "[$(date +%T)] $*" | tee -a "$R/room.log"; }
die() { log "FAIL: $*"; exit 1; }
# c W args: logosctl as member W, transcript in w.log, stdout returned
c() { local w=$1; shift; echo "\$ [$w $(date +%T)] logosctl $*" >> "$R/$(lc $w).log"; "$R/ctl.sh" $w "$@" 2>&1 | tee -a "$R/$(lc $w).log"; }
NOISE='^\s*$|Scanning|^\[[0-9-]+T|^Transaction data is|^WARNING: [Pp]roving in dev mode|^(Latest block is|Syncing to block|Synced to block)'
# m W args: msig as member W; output in out/last-W.out, exit code in RC
m() { local w=$1; shift; echo "\$ [$w $(date +%T)] msig $*" >> "$R/$(lc $w).log"
  LEE_WALLET_HOME_DIR="$R/w/$w" "$MSIG" "$@" > "$R/out/last-$w.out" 2>&1; RC=$?
  grep -v -E "$NOISE" "$R/out/last-$w.out" >> "$R/$(lc $w).log"; echo "(exit $RC)" >> "$R/$(lc $w).log"; }
ok() { [[ $RC -eq 0 ]] || die "$1: exit $RC ($(tail -2 "$R/out/last-$2.out"))"; }
has() { grep -q -- "$2" "$R/out/last-$1.out" || die "$1 output lacks: $2"; echo "  ASSERT output contains: $2" >> "$R/$(lc $1).log"; }
# msgs W G PREFIX OUTDIR: fetch room messages from others starting with PREFIX into OUTDIR/N.txt; prints count
msgs() { "$R/ctl.sh" $1 call chat_module get_messages "$2" 2>/dev/null | python3 -I -c '
import json,sys,os
pre,out=sys.argv[1],sys.argv[2]; os.makedirs(out,exist_ok=True)
try: r=json.loads(sys.stdin.read()).get("result") or []
except Exception: r=[]
n=0
for x in r:
  if x.get("from_self") or not x.get("content","").startswith(pre): continue
  open(f"{out}/{n}.txt","w").write(x["content"]); open(f"{out}/{n}.meta","w").write(json.dumps({k:x[k] for k in ("sender","timestamp_ms")})); n+=1
print(n)' "$3" "$4"; }
# post W G PREFIX FILE: send PREFIX+base64(FILE) into the room (retry on success:false)
post() { local body="$3$(base64 < "$4" | tr -d '\n')"; echo "  (posting ${#body} chars)" >> "$R/$(lc $1).log"
  for i in 1 2 3 4 5; do c $1 call chat_module send_message "$2" "$body" | grep -q '"success":true' && return 0; sleep 5; done; die "$1 send_message failed"; }
unb64() { python3 -I -c 'import sys,base64;t=open(sys.argv[1]).read();sys.stdout.buffer.write(base64.b64decode(t[len(sys.argv[2]):]))' "$1" "$2"; }
bal() { curl -s -X POST -H 'content-type: application/json' "http://127.0.0.1:$PORT" -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccountBalance\",\"params\":[\"$1\"]}"; }

SEQ_PID=
cleanup() {
  for w in A B C; do [[ -f "$R/out/watch-$w.pid" ]] && kill "$(cat "$R/out/watch-$w.pid")" 2>/dev/null; "$R/ctl.sh" $w daemon stop >> "$R/$(lc $w).log" 2>&1; done
  [[ -n "$SEQ_PID" ]] && kill "$SEQ_PID" 2>/dev/null; log "cleanup: daemons stopped, sequencer $SEQ_PID killed"
}
trap cleanup EXIT

log "run3 LEZ v0.4.0-rc1 1806e87; logosctl $("$R/ctl.sh" A --version 2>&1 | head -1); RISC0_DEV_MODE=1; sequencer :$PORT"

# ---- 0. chain: standalone LEZ sequencer, three wallets, 2-of-3 vault, funded (as scripts/cli-walkthrough.sh)
PAYER_PUB="$("$MSIG" keygen "$R/net/payer.key")"
python3 -I - "$ROOT/vendor/lez/lez/sequencer/service/networks/local.json" "$R/net/seq-config.json" "$PAYER_PUB" <<'PY'
import json, sys
c = json.load(open(sys.argv[1])); c["block_create_timeout"] = "3s"
c["genesis"]["actions"].append({"supply_account": {"recipient": {"identified": {"key": sys.argv[3]}}, "balance": 10_000_000_000_000}})
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
for w in A B C; do python3 -I - "$ROOT/vendor/lez/lez/wallet/configs/debug/wallet_config.json" "$R/w/$w/wallet_config.json" "$PORT" <<'PY'
import json, sys
c = json.load(open(sys.argv[1])); c["sequencer"]["sequencer_addr"] = f"http://127.0.0.1:{sys.argv[3]}"
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
done
"$SEQ" setup "$R/net/seqhome" --config "$R/net/seq-config.json" --funding-public-key 2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26 --create-channel > "$R/net/seq-setup.log" 2>&1
"$SEQ" start "$R/net/seqhome" --rpc-listen-address "127.0.0.1:$PORT" --metrics-listen-address 127.0.0.1:0 > "$R/net/sequencer.log" 2>&1 &
SEQ_PID=$!; echo $SEQ_PID > "$R/out/seq.pid"
for _ in $(seq 60); do [[ "$(lsof -t -nP -iTCP:$PORT -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] && break; sleep 1; done
[[ "$(lsof -t -nP -iTCP:$PORT -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] || die "sequencer not listening"
log "sequencer pid $SEQ_PID on :$PORT"
declare -A PK
for w in A B C; do PK[$w]=$("$MSIG" keygen "$R/w/$w/member.key"); echo "member key $w ${PK[$w]}" >> "$R/$(lc $w).log"; done
"$MSIG" keygen "$R/net/recipient.key" > /dev/null
for w in A B C; do m $w setup --payer-key "$R/net/payer.key"; ok setup $w; done
m A create --members "${PK[A]},${PK[B]},${PK[C]}" --threshold 2; ok create A; has A "threshold 2 of 3"
m B join "$R/w/A/msig-vault.json"; ok join B   # shortcut: vault file handed over on disk, not via the room (it is a secret; sealed distribution is out of scope)
m C join "$R/w/A/msig-vault.json"; ok join C
m A fund 1000 --payer-key "$R/net/payer.key"; ok fund A
m A fund 1 --payer-key "$R/net/payer.key" --to-key "$R/net/recipient.key"; ok fund A
RCPT="$(grep -o 'public account [A-Za-z0-9]*' "$R/out/last-A.out" | awk '{print $3}')"
m B status; ok status B; has B "balance 1000"
log "vault funded 1000; recipient $RCPT; rpc balance before: $(bal "$RCPT")"

# ---- 1. three logos daemons (native, logos.dev), chat_module + delivery_module 0.3.0 release lgx by path
for w in A B C; do c $w daemon start --detach; done
for w in A B C; do c $w install "$LGX/delivery_module-0.3.0.lgx" -y >/dev/null; c $w install "$LGX/chat_module-0.3.0.lgx" -y >/dev/null; c $w module load chat_module; done
start_chat() { local w=$1; "$R/ctl.sh" $w watch chat_module >> "$R/out/events-$w.log" 2>&1 & echo $! > "$R/out/watch-$w.pid"; sleep 1
  c $w call chat_module init 'json:{"delivery_preset":"logos.dev","log_level":"info"}' >/dev/null
  local t0; t0=$(ts); for i in $(seq 60); do c $w call chat_module status | grep -q '"delivery_state":"online"' && { log "TIMING $w online after $(echo "$(ts)-$t0"|bc)s"; return 0; }; sleep 2; done; die "$w not online"; }
for w in A B C; do start_chat $w; done
declare -A AD
for w in A B C; do AD[$w]=$(c $w call chat_module get_address | sed -n 's/.*"result":"\([0-9a-f]*\)".*/\1/p'); [[ -n "${AD[$w]}" ]] || die "no address $w"; done
log "chat addresses A=${AD[A]} B=${AD[B]} C=${AD[C]}"

# ---- 2. A creates the room, adds B and C; both must join (bounded 300 s)
G=$(c A call chat_module create_group_conversation msig-room run3 | sed -n 's/.*"value":"\([0-9a-f]*\)".*/\1/p'); [[ -n "$G" ]] || die "no group"
log "room $G"
c A call chat_module add_group_member "$G" "${AD[B]}" >/dev/null; c A call chat_module add_group_member "$G" "${AD[C]}" >/dev/null; tadd=$(ts)
declare -A J
for i in $(seq 100); do
  for w in B C; do [[ -z "${J[$w]:-}" ]] && "$R/ctl.sh" $w call chat_module list_conversations 2>/dev/null | grep -q "$G" && J[$w]=$(ts); done
  [[ -n "${J[B]:-}" && -n "${J[C]:-}" ]] && break; sleep 3; done
c A call chat_module list_group_members "$G"
[[ -n "${J[B]:-}" && -n "${J[C]:-}" ]] || die "join not within 300 s (B=${J[B]:-no} C=${J[C]:-no})"
log "TIMING join after add: B $(echo "${J[B]}-$tadd"|bc)s, C $(echo "${J[C]}-$tadd"|bc)s"

# ---- 3. A proposes 300 to the recipient and posts the envelope into the room
m A propose --to "$RCPT" --amount 300 --key "$R/w/A/member.key" --out "$R/w/A/prop.json"; ok propose A
post A "$G" "msig1:prop:" "$R/w/A/prop.json"; tprop=$(ts); log "A posted proposal ($(wc -c < "$R/w/A/prop.json") B envelope)"

# ---- 4. B and C receive it from the room, show (asserted), approve, post back; B also posts a forged approval
recv() { local w=$1 pre=$2 want=$3 dir=$4 lim=${5:-120}; local t0; t0=$(ts)
  for i in $(seq $((lim/2))); do [[ "$(msgs $w "$G" "$pre" "$dir")" -ge $want ]] && { log "TIMING $w got $want x '$pre' after $(echo "$(ts)-$t0"|bc)s of polling"; return 0; }; sleep 2; done; return 1; }
for w in B C; do
  recv $w "msig1:prop:" 1 "$R/out/rx-$w" || die "$w did not receive the proposal"
  unb64 "$R/out/rx-$w/0.txt" "msig1:prop:" > "$R/w/$w/prop.json"
  cmp -s "$R/w/$w/prop.json" "$R/w/A/prop.json" && echo "  received bytes identical to A's file" >> "$R/$(lc $w).log"
  echo "  room sender $(cat "$R/out/rx-$w/0.meta")" >> "$R/$(lc $w).log"
  m $w show "$R/w/$w/prop.json"; ok show $w; has $w "recipient $RCPT"; has $w "amount    300"; has $w "native-token"
  m $w approve "$R/w/$w/prop.json" --key "$R/w/$w/member.key"; ok approve $w
  post $w "$G" "msig1:approve:" "$R/w/$w/prop.json"; log "$w posted approval"
done
# forged: B's member key signs an approval over DIFFERENT bytes (its own 999 proposal), grafted onto the real envelope
m B propose --to "$RCPT" --amount 999 --key "$R/w/B/member.key" --out "$R/w/B/other.json"; ok propose B
m B approve "$R/w/B/other.json" --key "$R/w/B/member.key"; ok approve B
python3 -I - "$R/w/B/other.json" "$R/out/rx-B/0.txt" "$R/w/B/forged.json" <<'PY'
import json, sys, base64
o = json.load(open(sys.argv[1])); t = open(sys.argv[2]).read()
e = json.loads(base64.b64decode(t[len("msig1:prop:"):]))
e["sigs"].append([s for s in o["sigs"] if s["role"] == "approve"][0])
json.dump(e, open(sys.argv[3], "w"))
PY
echo "  forged.json = received proposal + B approve sig taken from other.json (amount 999)" >> "$R/b.log"
post B "$G" "msig1:approve:" "$R/w/B/forged.json"; log "B posted FORGED approval"

# ---- 5. A collects approvals from the room, validates each with msig show, merges the valid ones, executes
recv A "msig1:approve:" 3 "$R/out/rx-A" || die "A did not get 3 approvals"
log "TIMING proposal post -> 3 approvals at A: $(echo "$(ts)-$tprop"|bc)s"
cp "$R/w/A/prop.json" "$R/w/A/merged.json"
for f in "$R"/out/rx-A/*.txt; do
  unb64 "$f" "msig1:approve:" > "$R/w/A/cand.json"
  python3 -I - "$R/w/A/prop.json" "$R/w/A/cand.json" "$R/w/A/one.json" <<'PY' || { echo "  $f: signed bytes differ from our proposal: dropped" >> "$R/a.log"; continue; }
import json, sys
p, c = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
assert c["signed"] == p["signed"]
p["sigs"] += [s for s in c["sigs"] if s["role"] == "approve" and s not in p["sigs"]]
json.dump(p, open(sys.argv[3], "w"))
PY
  echo "  room message $(basename "$f") from $(cat "${f%.txt}.meta")" >> "$R/a.log"
  m A show "$R/w/A/one.json"
  if [[ $RC -ne 0 ]] && grep -q INVALID "$R/out/last-A.out"; then log "A: approval $(basename "$f") INVALID by msig show: dropped"; cp "$R/w/A/one.json" "$R/w/A/forged-seen.json"; continue; fi
  ok "show approval" A
  python3 -I - "$R/w/A/merged.json" "$R/w/A/one.json" <<'PY'
import json, sys
m, o = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
m["sigs"] += [s for s in o["sigs"] if s not in m["sigs"]]; json.dump(m, open(sys.argv[1], "w"))
PY
  log "A: approval $(basename "$f") valid: merged"
done
# the forged approval does not count on execute either: forged + C's valid one = 1 valid approval, refused before proving
[[ -f "$R/w/A/forged-seen.json" ]] || die "forged approval was not flagged INVALID by show"
python3 -I - "$R/w/A/forged-seen.json" "$R/w/A/merged.json" "$R/w/A/forged-plus-c.json" "${PK[C]}" <<'PY'
import json, sys
f, m = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
f["sigs"] += [s for s in m["sigs"] if s["key"] == sys.argv[4] and s["role"] == "approve"]; json.dump(f, open(sys.argv[3], "w"))
PY
m A execute "$R/w/A/forged-plus-c.json" --key "$R/w/A/member.key"; [[ $RC -ne 0 ]] || die "forged+C executed"; has A "1 valid approval"; has A "BELOW THRESHOLD"
log "A: execute(proposal + forged B + valid C) refused: BELOW THRESHOLD, 1 valid approval"
m A show "$R/w/A/merged.json"; ok "show merged" A
m A execute "$R/w/A/merged.json" --key "$R/w/A/member.key"; ok execute A; has A "2 valid approval"; has A "balance 700"; has A "nonce 1"
m C status; ok status C; has C "balance 700"
log "EXECUTED: vault 1000 -> 700 (nonce 1); recipient rpc balance after: $(bal "$RCPT")"

# ---- 6. async: C offline while A proposes (50) and B approves; then C comes back
c C daemon stop; kill "$(cat "$R/out/watch-C.pid")" 2>/dev/null; rm -f "$R/out/watch-C.pid"; log "C daemon stopped"
m A propose --to "$RCPT" --amount 50 --key "$R/w/A/member.key" --out "$R/w/A/prop2.json"; ok propose A
post A "$G" "msig1:prop2:" "$R/w/A/prop2.json"; log "A posted proposal 2 (C offline)"
recv B "msig1:prop2:" 1 "$R/out/rx2-B" || die "B did not receive proposal 2"
unb64 "$R/out/rx2-B/0.txt" "msig1:prop2:" > "$R/w/B/prop2.json"
m B show "$R/w/B/prop2.json"; ok show B; has B "amount    50"
m B approve "$R/w/B/prop2.json" --key "$R/w/B/member.key"; ok approve B
post B "$G" "msig1:approve2:" "$R/w/B/prop2.json"; log "B approved proposal 2"
c C daemon start --detach; c C module load chat_module; start_chat C
c C call chat_module get_address; c C call chat_module list_conversations
if recv C "msig1:prop2:" 1 "$R/out/rx2-C" 180; then log "ASYNC: C received proposal 2 after restart"; else log "ASYNC: C did NOT receive proposal 2 within 180 s after restart"; fi
c C call chat_module list_conversations; c C call chat_module get_messages "$G"
# the quorum does not need C: A approves itself, takes B's approval from the room, executes
recv A "msig1:approve2:" 1 "$R/out/rx2-A" || die "A did not get B's approval 2"
unb64 "$R/out/rx2-A/0.txt" "msig1:approve2:" > "$R/w/A/prop2.json"
m A approve "$R/w/A/prop2.json" --key "$R/w/A/member.key"; ok approve A
m A execute "$R/w/A/prop2.json" --key "$R/w/A/member.key"; ok execute A; has A "balance 650"; has A "nonce 2"
log "EXECUTED round 2 without C: vault 700 -> 650; recipient rpc balance: $(bal "$RCPT")"
log "PASS"
