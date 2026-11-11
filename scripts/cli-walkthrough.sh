#!/usr/bin/env bash
# Scripted 2-of-3 walkthrough of the `msig` operator CLI on a fresh standalone LEZ v0.4.0-rc1
# sequencer (dev mode, loopback only): three operators in separate wallet homes exchange envelope
# FILES. create, fund, propose, 2 approvals, show (decoded output asserted), execute, tamper refused,
# config remove member, stale approval rejected; image v2: spending limit + Spend (over-limit
# refused), time lock + Queue -> ExecuteQueued (immediate execute and early execute refused).
# Transcript: evidence/cli-walkthrough.txt. Dev proofs only (RISC0_DEV_MODE=1); keys are generated
# fresh into target/cli-walkthrough. RISC0_USE_DOCKER=1 by default: deploys the frozen image v2.1.
#
# Env: MSIG_LEZ_PORT (default 3081; must be >= 3080), LEZ_SEQUENCER_BIN (sequencer_service built
# from LEZ 1806e87), MSIG_CLI_HOME (default target/cli-walkthrough, wiped on start).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${MSIG_LEZ_PORT:-3081}"
SEQ="${LEZ_SEQUENCER_BIN:?set LEZ_SEQUENCER_BIN to a sequencer_service built from LEZ 1806e87}"
H="${MSIG_CLI_HOME:-$ROOT/target/cli-walkthrough}"
OUT="$ROOT/evidence/cli-walkthrough.txt"
(( PORT >= 3080 )) || { echo "port $PORT: use 3080 or above" >&2; exit 2; }
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then echo "port $PORT is busy" >&2; exit 2; fi
[[ -x "$SEQ" ]] || { echo "no sequencer_service at $SEQ" >&2; exit 2; }

export PATH="$PATH:$HOME/.risc0/bin" RISC0_DEV_MODE=1 RISC0_USE_DOCKER="${RISC0_USE_DOCKER:-1}"
cd "$ROOT"
cargo build -q -p msig-cli
# reproducibility anchor for the transcript header: the guest image id of this build (same env as msig)
IMG="$(cargo run -q -p msig-methods --example image_id | sed -n 's/^msig //p')"
MSIG="$ROOT/target/debug/msig"

rm -rf "$H"; mkdir -p "$H/wallet" "$H/net" "$H/op1" "$H/op2" "$H/op3"
PAYER_PUB="$("$MSIG" keygen "$H/payer.key")"
python3 -I - "$ROOT/vendor/lez/lez/sequencer/service/networks/local.json" "$H/net/seq-config.json" "$PAYER_PUB" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["block_create_timeout"] = "3s"
c["genesis"]["actions"].append({"supply_account": {"recipient": {"identified": {"key": sys.argv[3]}}, "balance": 10_000_000_000_000}})
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
for i in 1 2 3; do
  python3 -I - "$ROOT/vendor/lez/lez/wallet/configs/debug/wallet_config.json" "$H/op$i/wallet_config.json" "$PORT" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["sequencer"]["sequencer_addr"] = f"http://127.0.0.1:{sys.argv[3]}"
json.dump(c, open(sys.argv[2], "w"), indent=1)
PY
done
"$SEQ" setup "$H/net/seqhome" --config "$H/net/seq-config.json" \
  --funding-public-key 2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26 --create-channel >"$H/net/seq-setup.log" 2>&1
"$SEQ" start "$H/net/seqhome" --rpc-listen-address "127.0.0.1:$PORT" --metrics-listen-address 127.0.0.1:0 >"$H/net/sequencer.log" 2>&1 &
SEQ_PID=$!
trap 'kill $SEQ_PID 2>/dev/null || true' EXIT
for _ in $(seq 60); do [[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] && break; sleep 1; done
[[ "$(lsof -t -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null)" == "$SEQ_PID" ]] || { echo "sequencer $SEQ_PID not listening on $PORT; see $H/net/sequencer.log" >&2; exit 2; }

NOISE='^\s*$|Scanning|^\[[0-9-]+T|^Transaction data is|^WARNING: [Pp]roving in dev mode|^(Latest block is|Syncing to block|Synced to block)'
# m N args...: run `msig args` as operator N; output in $H/last.out, exit code in $RC
m() {
  local n=$1; shift
  echo; echo "\$ [operator $n] msig $*"
  set +e; LEE_WALLET_HOME_DIR="$H/op$n" "$MSIG" "$@" >"$H/last.out" 2>&1; RC=$?; set -e
  grep -v -E "$NOISE" "$H/last.out" || true
}
ok()   { [[ $RC -eq 0 ]] || { echo "FAIL: exit $RC"; exit 1; }; }
fails(){ [[ $RC -ne 0 ]] || { echo "FAIL: expected a refusal"; exit 1; }; }
has()  { grep -q -- "$1" "$H/last.out" || { echo "FAIL: output lacks: $1"; exit 1; }; echo "  ASSERT output contains: $1"; }

{
  echo "# scripts/cli-walkthrough.sh  $(date -u +%FT%TZ)  image msig $IMG"
  echo "# LEZ v0.4.0-rc1 1806e87 sequencer on 127.0.0.1:$PORT, RISC0_DEV_MODE=1 (proofs are dev-mode receipts)"
  echo "# three operators = three wallet homes; envelopes move as files between them"

  echo; echo "## 0. keys and per-operator setup (program deployed once, each operator gets a funded private fee account)"
  for i in 1 2 3; do PUB[$i]=$("$MSIG" keygen "$H/op$i/member.key"); echo "operator $i member key: ${PUB[$i]}"; done
  "$MSIG" keygen "$H/recipient.key" >/dev/null
  for i in 1 2 3; do m $i setup --payer-key "$H/payer.key"; ok; done

  echo; echo "## 1. create the 2-of-3 private multisig (operator 1) and hand the vault file to 2 and 3"
  m 1 create --members "${PUB[1]},${PUB[2]},${PUB[3]}" --threshold 2 --perms 15; ok; has "threshold 2 of 3"
  m 2 join "$H/op1/msig-vault.json"; ok
  m 3 join "$H/op1/msig-vault.json"; ok

  echo; echo "## 2. fund the vault and open a public recipient"
  m 1 fund 1000 --payer-key "$H/payer.key"; ok
  m 1 fund 1 --payer-key "$H/payer.key" --to-key "$H/recipient.key"; ok
  RCPT="$(grep -o 'public account [A-Za-z0-9]*' "$H/last.out" | awk '{print $3}')"
  m 2 status; ok; has "balance 1000"

  echo; echo "## 3. propose a payout of 300 (operator 1), approve (2, 3): the file travels"
  m 1 propose --to "$RCPT" --amount 300 --key "$H/op1/member.key" --out "$H/prop.json"; ok
  cp "$H/prop.json" "$H/op2/prop.json"; cp "$H/prop.json" "$H/op3/prop.json"
  m 2 show "$H/op2/prop.json"; ok
  has "native-token"; has "recipient $RCPT"; has "amount    300"; has "nonce        0"; has "epoch        0"; has "approve digest"
  m 2 approve "$H/op2/prop.json" --key "$H/op2/member.key"; ok
  cp "$H/op2/prop.json" "$H/op3/prop.json"      # operator 3 receives the file with 2's signature
  m 3 approve "$H/op3/prop.json" --key "$H/op3/member.key"; ok
  m 3 show "$H/op3/prop.json"; ok; has "approve   *"; has "propose"

  echo; echo "## 4. a tampered envelope is refused by show and by execute (amount 300 -> 700 in the signed bytes)"
  python3 -I - "$H/op3/prop.json" "$H/tampered.json" <<'PY'
import json, sys
e = json.load(open(sys.argv[1])); s = bytearray(bytes.fromhex(e["signed"]))
s[-17] ^= 0x0b  # low byte of the u128 amount: 300 -> 311
e["signed"] = bytes(s).hex(); json.dump(e, open(sys.argv[2], "w"))
PY
  m 3 show "$H/tampered.json"; fails; has "TAMPERED"
  m 3 execute "$H/tampered.json" --key "$H/op3/member.key"; fails; has "TAMPERED"

  echo; echo "## 5. a second payout is proposed now (and approved) so it can go stale below"
  m 1 propose --to "$RCPT" --amount 50 --key "$H/op1/member.key" --out "$H/old.json" >/dev/null; ok
  # old.json is at nonce 0 as well: it dies as soon as the first payout lands
  m 2 approve "$H/old.json" --key "$H/op2/member.key" >/dev/null; ok
  m 3 approve "$H/old.json" --key "$H/op3/member.key" >/dev/null; ok

  echo; echo "## 6. execute the 300 payout (operator 3 proves and submits; progress and elapsed time are printed)"
  m 3 execute "$H/op3/prop.json" --key "$H/op3/member.key"; ok
  has "\[prove\] done in"; has "balance 700"; has "nonce 1"
  m 2 status; ok; has "balance 700"

  echo; echo "## 7. the older proposal (nonce 0) is stale now: refused before any proving, with a clear message"
  m 2 execute "$H/old.json" --key "$H/op2/member.key"; fails; has "STALE"

  echo; echo "## 8. config: remove member 3 (needs 2 approvals like any action); a payout proposed before it goes stale"
  m 1 propose --to "$RCPT" --amount 40 --key "$H/op1/member.key" --out "$H/pre-config.json" >/dev/null; ok
  m 2 approve "$H/pre-config.json" --key "$H/op2/member.key" >/dev/null; ok
  m 3 approve "$H/pre-config.json" --key "$H/op3/member.key" >/dev/null; ok
  m 1 config remove-member "${PUB[3]}" --key "$H/op1/member.key" --out "$H/cfg.json"; ok; has "CONFIG remove member ${PUB[3]}"
  m 1 approve "$H/cfg.json" --key "$H/op1/member.key" >/dev/null; ok
  m 2 approve "$H/cfg.json" --key "$H/op2/member.key" >/dev/null; ok
  m 2 execute "$H/cfg.json" --key "$H/op2/member.key"; ok; has "threshold 2 of 2"; has "epoch 1"
  m 3 status; ok; has "threshold 2 of 2"

  echo; echo "## 9. approvals signed before the epoch change are rejected; the removed member cannot act"
  m 1 execute "$H/pre-config.json" --key "$H/op1/member.key"; fails; has "STALE"
  m 1 propose --to "$RCPT" --amount 40 --key "$H/op3/member.key" --out "$H/by-removed.json"; fails; has "not a member"

  echo; echo "## 10. image v2: spending limit 150 per day, 1 spend signature (a config change: same quorum)"
  m 1 config set-limit 150 --period-ms 86400000 --threshold 1 --key "$H/op1/member.key" --out "$H/limit.json"; ok
  has "CONFIG set spending limit 150 per 86400000 ms, 1 spend signature"
  m 1 approve "$H/limit.json" --key "$H/op1/member.key" >/dev/null; ok
  m 2 approve "$H/limit.json" --key "$H/op2/member.key" >/dev/null; ok
  m 2 execute "$H/limit.json" --key "$H/op2/member.key"; ok; has "limit 150 per 86400000 ms (1 sig)"; has "epoch 2  nonce 3"

  echo; echo "## 11. Spend 100 under the limit: one spend signature, no proposal or approvals; a second 100 is over the limit"
  m 1 spend --to "$RCPT" --amount 100 --key "$H/op1/member.key" --out "$H/spend.json"; ok
  cp "$H/spend.json" "$H/op2/spend.json"
  m 2 show "$H/op2/spend.json"; ok; has "SPEND under the spending limit"; has "recipient  $RCPT"; has "amount     100"; has "spend    *[0-9a-f]*  valid"
  m 2 execute "$H/op2/spend.json" --key "$H/op2/member.key"; ok; has "balance 600"; has "spent 100 in period"; has "nonce 3"; has "spend nonce 1"
  m 1 spend --to "$RCPT" --amount 100 --key "$H/op1/member.key" --out "$H/spend2.json" >/dev/null; ok
  m 2 execute "$H/spend2.json" --key "$H/op2/member.key"; fails; has "OverLimit"

  echo; echo "## 12. time lock 5 s (config); from now on actions go queue -> execute-queued"
  m 1 config set-time-lock 5000 --key "$H/op1/member.key" --out "$H/tl.json"; ok; has "CONFIG set time lock 5000 ms"
  m 1 approve "$H/tl.json" --key "$H/op1/member.key" >/dev/null; ok
  m 2 approve "$H/tl.json" --key "$H/op2/member.key" >/dev/null; ok
  m 2 execute "$H/tl.json" --key "$H/op2/member.key"; ok; has "time lock 5000 ms"; has "epoch 3  nonce 4"

  echo; echo "## 13. payout 100 under the time lock: immediate execute refused, Queue, early execute refused, ExecuteQueued after eta"
  m 1 propose --to "$RCPT" --amount 100 --key "$H/op1/member.key" --out "$H/q.json" >/dev/null; ok
  m 1 approve "$H/q.json" --key "$H/op1/member.key" >/dev/null; ok
  m 2 approve "$H/q.json" --key "$H/op2/member.key" >/dev/null; ok
  m 2 execute "$H/q.json" --key "$H/op2/member.key"; fails; has "TimeLockMismatch"
  m 1 queue "$H/q.json" --key "$H/op1/member.key"; ok; has "queued. vault now"; has "queue eta"; has "balance 600"
  ETA="$(grep -o 'queue eta [0-9]*' "$H/last.out" | awk '{print $3}')"
  m 2 execute-queued "$H/q.json" --key "$H/op2/member.key"; fails; has "TOO EARLY"
  WAIT=$(( (ETA - $(date +%s) * 1000) / 1000 + 2 )); (( WAIT > 0 )) || WAIT=0
  echo; echo "(waiting ${WAIT} s for eta $ETA)"; sleep "$WAIT"
  m 2 execute-queued "$H/q.json" --key "$H/op2/member.key"; ok; has "balance 500"; has "queue empty"; has "nonce 5"
  echo; echo "PASS"
} 2>&1 | tee "$OUT"
grep -q '^PASS$' "$OUT"
