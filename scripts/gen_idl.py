#!/usr/bin/env python3
"""Generate / check the SPEL-format IDL (idl/msig.idl.json) of the multisig program.

Types, variants, field order, error codes and constants are PARSED from crates/msig-core/src/lib.rs;
the lee_core types the message embeds (AccountId, Actor, keys) are hand-listed and asserted against
vendor/lez.

    python3 scripts/gen_idl.py           # rewrite idl/msig.idl.json
    python3 scripts/gen_idl.py --check   # exit 1 on drift (CI)
Python 3 stdlib only.
"""
import json, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = re.sub(r"//[^\n]*", "", (ROOT / "crates/msig-core/src/lib.rs").read_text())
CORE = ROOT / "vendor/lez/lee/state_machine/core/src"
PRIM = {"u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "bool", "String"}
B32 = {"array": ["u8", 32]}

# lee_core types embedded in the message; (file, regex that must match the vendored source)
EXTERN = {
    "AccountId": ([("value", B32)], "account.rs", r"pub struct AccountId \{\s*value: \[u8; 32\],\s*\}"),
    "Actor": ([("account_id", {"defined": "AccountId"}), ("program_account_id", {"defined": "AccountId"})], "account.rs",
              r"pub struct Actor \{\s*pub account_id: AccountId,\s*pub program_account_id: AccountId,\s*\}"),
    "NullifierPublicKey": ([("0", B32)], "nullifier.rs", r"pub struct NullifierPublicKey\(pub \[u8; 32\]\);"),
    "ViewingPublicKey": ([("0", {"vec": "u8"})], "encryption/shared_key_derivation.rs",
                         r"pub struct MlKem768EncapsulationKey\(Vec<u8>\);\s*pub type ViewingPublicKey = MlKem768EncapsulationKey;"),
}
ERR_MSG = {
    "Malformed": "Undecodable instruction or state",
    "AlreadyInitialized": "Create on an account that already has state",
    "Uninitialized": "Execute on an account with no state",
    "WrongAccount": "Account is not the PDA derived from (program, seed, keys)",
    "InvalidConfig": "Violates the config invariants (empty or >MAX_MEMBERS, bad or duplicate key, perms 0 or outside ALL_PERMS, time_lock above MAX_TIME_LOCK, threshold 0 or above the VOTE count, no INITIATE or EXECUTE member, limit with threshold 0 or above the SPEND count or period 0)",
    "BelowThreshold": "Fewer approvals than the threshold",
    "UnsortedSigners": "Approvals not in strictly ascending member index order",
    "NotMember": "Signer index (or removed key) is not a member",
    "MissingPermission": "Member lacks the permission bit the role needs",
    "BadSignature": "BIP-340 signature does not verify over the recomputed digest",
    "AuthorityOnForeignAccount": "with_authority call targets an account other than the multisig",
    "Reentry": "Action::Call targets this program",
    "Overflow": "nonce, config_epoch, spend counter or time arithmetic overflow",
    "UnsupportedVersion": "State version is not VERSION (a v1 state is rejected, not migrated)",
    "TimeLockMismatch": "Immediate Execute of a non-Cancel action under a time lock, or Queue without one",
    "QueueOccupied": "The queue slot holds an entry that has not expired before t",
    "NotQueued": "Nothing queued, or the queued digest does not match (other action, nonce or epoch)",
    "BadWindow": "Empty validity window (t + time_lock >= expiry, or the timestamp window is invalid)",
    "NoLimit": "Spend without a spending limit",
    "OverLimit": "Spend would exceed the limit amount in this period",
    "StalePeriod": "Spend period p is before the current spend period",
    "QueuerNotProposer": "Queue by a member other than the proposer (approvers do not sign t; only the proposer picks it)",
}
INSTR_DOC = {
    "Create": "Root -> multisig actor (V, P). Initialises state; V must equal the PDA derived from seed = sha256(pad32('LEZ-MSIG/v1/seed') || sha256(borsh(cfg)) || salt) under this program (for_public_pda, or for_private_pda with keys). The seed is computed from cfg, not a plain argument path, so no `pda` seeds are listed.",
    "Execute": "Root -> multisig actor (V, P). Immediate execution: proposer, >= threshold approvals (strictly ascending index), executor, each BIP-340 over the digests of docs/ARCHITECTURE.md 2.4. Valid while timestamp < expiry. Bumps nonce; Config also bumps config_epoch. Under a time lock only Cancel executes immediately.",
    "Queue": "Root -> multisig actor (V, P). Starts the time lock: the Execute quorum (proposer, >= threshold approvals) plus the proposer's own /queue signature over body || t (queuer index = proposer index). Valid while timestamp < t (and >= the old entry's expiry if the slot is taken); needs time_lock != 0 and t + time_lock < expiry. Stores the /queue digest in the single queue slot.",
    "ExecuteQueued": "Root -> multisig actor (V, P). Executes the queued action: an EXECUTE member's /execute signature over body(queued.expiry) || queued.t; the action must hash to the queued digest. Valid in [t + time_lock, expiry). Bumps nonce, clears the slot.",
    "Spend": "Root -> multisig actor (V, P). Native transfer V -> to under the spending limit: >= limit.threshold SPEND members (strictly ascending index) sign /spend over epoch || spend.nonce || p || to || amount. Valid in [p*period, (p+1)*period). No proposal, no time lock; does not touch the action nonce.",
}
ITEMS = {n: (k, b) for k, n, b in re.findall(r"^pub (struct|enum) (\w+) \{(.*?)^\}", SRC, re.S | re.M)}


def split_top(s):
    out, d, cur = [], 0, ""
    for ch in s:
        d += ch in "<([{"
        d -= ch in ">)]}"
        if ch == "," and d == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def strip_attrs(s):
    return re.sub(r"#\[[^\]]*\]", "", s).strip()


class Conv:
    def __init__(self):
        self.types = {}

    def ty(self, r):
        r = r.replace(" ", "")
        if r.startswith("Vec<"):
            return {"vec": self.ty(r[4:-1])}
        if r.startswith("Option<"):
            return {"option": self.ty(r[7:-1])}
        if r.startswith("["):
            inner, n = r[1:-1].rsplit(";", 1)
            return {"array": [self.ty(inner), int(n)]}
        if r.startswith("("):
            parts = [self.ty(p) for p in split_top(r[1:-1])]
            name = "Tuple_" + "_".join(p if isinstance(p, str) else p["defined"] for p in parts)
            self.types[name] = {"name": name, "kind": "struct", "fields": [{"name": str(i), "type": p} for i, p in enumerate(parts)]}
            return {"defined": name}
        if r in PRIM:
            return r
        if r in EXTERN:
            fields, f, rx = EXTERN[r]
            assert re.search(rx, re.sub(r"//[^\n]*|#\[[^\]]*\]", "", (CORE / f).read_text())), f"vendored {r} changed: update EXTERN in gen_idl.py"
            self.types[r] = {"name": r, "kind": "struct", "fields": [{"name": n, "type": self.ext(t)} for n, t in fields]}
            for _, t in fields:
                if isinstance(t, dict) and "defined" in t:
                    self.ty(t["defined"])
            return {"defined": r}
        if r in ITEMS:
            self.define(r)
            return {"defined": r}
        raise SystemExit(f"unknown type {r!r}")

    def ext(self, t):
        return {"vec": t["vec"]} if isinstance(t, dict) and "vec" in t else t

    def fields(self, body):
        out = []
        for f in split_top(strip_attrs(body)):
            n, t = strip_attrs(f).removeprefix("pub ").split(":", 1)
            out.append({"name": n.strip(), "type": self.ty(t.strip())})
        return out

    def define(self, name):
        if name in self.types:
            return
        self.types[name] = None
        kind, body = ITEMS[name]
        if kind == "struct":
            self.types[name] = {"name": name, "kind": "struct", "fields": self.fields(body)}
            return
        vs = []
        for v in split_top(strip_attrs(body)):
            name_, _, sf, tf = re.fullmatch(r"(\w+)\s*(\{(.*)\}|\((.*)\))?", strip_attrs(v), re.S).groups()
            fs = self.fields(sf) if sf is not None else [{"name": str(i), "type": self.ty(t)} for i, t in enumerate(split_top(tf))] if tf is not None else []
            vs.append({"name": name_, "fields": fs})
        self.types[name] = {"name": name, "kind": "enum", "variants": vs}


def errors():
    body = re.search(r"pub enum Error \{(.*?)\}", SRC, re.S).group(1)
    names, code = [], 0
    out = []
    for n, c in re.findall(r"(\w+)(?: = (\d+))?,", body):
        code = int(c) if c else code + 1
        out.append({"code": code, "name": n, "msg": ERR_MSG[n]})
    assert {e["name"] for e in out} == set(ERR_MSG), "Error enum changed: update ERR_MSG in gen_idl.py"
    return out


def build():
    cv = Conv()
    cv.define("Instruction")
    cv.define("Multisig")
    instrs = []
    for i, v in enumerate(cv.types["Instruction"]["variants"]):
        instrs.append({
            "name": re.sub(r"(?<!^)([A-Z])", r"_\1", v["name"]).lower(),
            "accounts": [{"name": "multisig", "writable": True, "signer": False, "init": v["name"] == "Create",
                          "visibility": ["public", "private_pda"]}],
            "args": v["fields"],
            "execution": {"public": True, "private_owned": False},
            "variant": v["name"],
            "variant_index": i,
            "doc": INSTR_DOC[v["name"]],
        })
    consts = {n: int(x) for n, x in re.findall(r"^pub const (\w+): (?:u8|u32|usize) = (\d+);", SRC, re.M)}
    consts.update({n: s for n, s in re.findall(r'^pub const (TAG_\w+): &str = "([^"]+)";', SRC, re.M)})
    state = {k: v for k, v in cv.types["Multisig"].items() if k != "name"}
    return {
        "version": "0.1.0",
        "name": "lez_msig",
        "instructions": instrs,
        "accounts": [{"name": "Multisig", "type": state,
                      "doc": "Actor state of (V, P): borsh(Multisig), no tag byte. V is a public or private PDA of the program."}],
        "types": [cv.types[k] for k in sorted(cv.types)],
        "errors": errors(),
        "spec": "0.1.0",
        "metadata": {"name": "lez_msig", "version": "0.1.0"},
        "instruction_type": "msig_core::Instruction",
        "constants": consts,
        "notes": "SPEL-format IDL for LEZ v0.4 actor programs, generated by scripts/gen_idl.py from crates/msig-core. Wire format = borsh(Instruction), enum tag = variant_index (u8), not SPEL's sha256('global:name') discriminator. Failures exit with the Error code (u8). Keys outside SPEL's schema (variant_index, doc, constants, notes) are ignored by its serde types. SPEL's own generator and client targets v0.2.5 and were not run against v0.4.",
    }


def main():
    out = json.dumps(build(), indent=2) + "\n"
    p = ROOT / "idl/msig.idl.json"
    if "--check" in sys.argv:
        if not p.exists() or p.read_text() != out:
            print(f"DRIFT: {p} differs from the Rust source; run `python3 scripts/gen_idl.py`")
            sys.exit(1)
        print("ok: msig.idl.json")
    else:
        p.write_text(out)
        print(f"wrote {p}")


if __name__ == "__main__":
    main()
