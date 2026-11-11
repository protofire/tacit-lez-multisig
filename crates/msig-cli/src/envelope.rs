//! Proposal envelope (docs/CLI.md): one JSON file carrying the exact signed bytes
//! `vault(32) ‖ body(56) ‖ borsh(action)` plus the members' signatures over the §2.4 digests.
//! Everything `show` prints is decoded from `signed`, never from separate metadata.
//! A spend envelope (`"kind": "spend"`) carries `vault(32) ‖ spend body(72)` instead.

use anyhow::{Result, anyhow, bail, ensure};
use k256::schnorr::{Signature, SigningKey, VerifyingKey, signature::hazmat::PrehashVerifier};
use lee::AccountId;
use lee_core::native_token::{Message as Native, NATIVE_TOKEN_PROGRAM_ID};
use msig_core::{Action, ConfigOp, TAG_APPROVE, TAG_PROPOSE, TAG_SPEND, body, digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const ENVELOPE_VERSION: u8 = 1;
/// Advisory only: the program never sees it.
pub const TAG_REJECT: &str = "LEZ-MSIG/v1/reject";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Propose,
    Approve,
    Reject,
    /// `/spend` signature of a SPEND member (spend envelopes only).
    Spend,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Propose => "propose",
            Role::Approve => "approve",
            Role::Reject => "reject",
            Role::Spend => "spend",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Role::Propose => TAG_PROPOSE,
            Role::Approve => TAG_APPROVE,
            Role::Reject => TAG_REJECT,
            Role::Spend => TAG_SPEND,
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "propose" => Role::Propose,
            "approve" => Role::Approve,
            "reject" => Role::Reject,
            "spend" => Role::Spend,
            _ => bail!("unknown signature role {s:?}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvSig {
    pub role: Role,
    pub key: [u8; 32],
    pub sig: [u8; 64],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// `vault(32) ‖ body(56) ‖ borsh(action)`.
    pub signed: Vec<u8>,
    pub sigs: Vec<EnvSig>,
    /// Spend envelope: `signed = vault(32) ‖ spend body(72)`. Not signed itself, but the `/spend`
    /// tag is, and the other reading of the same bytes fails to decode (TAMPERED / length).
    pub spend: bool,
}

/// A spend envelope's signed bytes: `epoch ‖ spend nonce ‖ p ‖ to ‖ amount` (msig_core::spend_digest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendDecoded {
    pub vault: AccountId,
    pub epoch: u64,
    pub nonce: u64,
    pub p: u64,
    pub to: AccountId,
    pub amount: u128,
}

/// What the signed bytes say, after the action hash in the body is checked against the action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub vault: AccountId,
    pub epoch: u64,
    pub nonce: u64,
    pub expiry: u64,
    pub action: Action,
    pub body: [u8; 56],
}

#[derive(Serialize, Deserialize)]
struct WireSig {
    role: String,
    key: String,
    sig: String,
}

#[derive(Serialize, Deserialize)]
struct Wire {
    version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    signed: String,
    sigs: Vec<WireSig>,
}

fn arr<const N: usize>(h: &str, what: &str) -> Result<[u8; N]> {
    hex::decode(h)
        .map_err(|e| anyhow!("{what}: bad hex: {e}"))?
        .try_into()
        .map_err(|_| anyhow!("{what}: expected {N} bytes"))
}

impl Envelope {
    pub fn new(vault: &AccountId, epoch: u64, nonce: u64, expiry: u64, action: &Action) -> Self {
        let mut signed = vault.value().to_vec();
        signed.extend(body(epoch, nonce, expiry, action));
        signed.extend(borsh::to_vec(action).expect("borsh"));
        Self {
            signed,
            sigs: Vec::new(),
            spend: false,
        }
    }

    pub fn new_spend(vault: &AccountId, epoch: u64, nonce: u64, p: u64, to: &AccountId, amount: u128) -> Self {
        let mut signed = vault.value().to_vec();
        for f in [epoch, nonce, p] {
            signed.extend(f.to_le_bytes());
        }
        signed.extend(to.value());
        signed.extend(amount.to_le_bytes());
        Self {
            signed,
            sigs: Vec::new(),
            spend: true,
        }
    }

    pub fn to_json(&self) -> String {
        let w = Wire {
            version: ENVELOPE_VERSION,
            kind: self.spend.then(|| "spend".into()),
            signed: hex::encode(&self.signed),
            sigs: self
                .sigs
                .iter()
                .map(|s| WireSig {
                    role: s.role.name().into(),
                    key: hex::encode(s.key),
                    sig: hex::encode(s.sig),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&w).expect("json") + "\n"
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let w: Wire = serde_json::from_str(s).map_err(|e| anyhow!("envelope is not valid: {e}"))?;
        ensure!(
            w.version == ENVELOPE_VERSION,
            "envelope version {} not supported (this build reads {ENVELOPE_VERSION})",
            w.version
        );
        let spend = match w.kind.as_deref() {
            None => false,
            Some("spend") => true,
            Some(k) => bail!("unknown envelope kind {k:?}"),
        };
        Ok(Self {
            spend,
            signed: hex::decode(&w.signed).map_err(|e| anyhow!("signed: bad hex: {e}"))?,
            sigs: w
                .sigs
                .iter()
                .map(|s| {
                    Ok(EnvSig {
                        role: Role::parse(&s.role)?,
                        key: arr(&s.key, "sig key")?,
                        sig: arr(&s.sig, "sig")?,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }

    /// Splits `signed` and refuses it unless `body` commits to exactly the embedded action.
    pub fn decode(&self) -> Result<Decoded> {
        ensure!(!self.spend, "this is a spend envelope: use the spend flow");
        ensure!(self.signed.len() > 88, "signed bytes too short");
        let vault = AccountId::new(self.signed[..32].try_into().unwrap());
        let b: [u8; 56] = self.signed[32..88].try_into().unwrap();
        let raw = &self.signed[88..];
        let action: Action = borsh::from_slice(raw)
            .map_err(|e| anyhow!("TAMPERED: embedded action does not decode: {e}"))?;
        ensure!(
            Sha256::digest(raw).as_slice() == &b[24..],
            "TAMPERED: the signed body commits to a different action than the one embedded"
        );
        let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Ok(Decoded {
            vault,
            epoch: u(0),
            nonce: u(8),
            expiry: u(16),
            action,
            body: b,
        })
    }

    pub fn decode_spend(&self) -> Result<SpendDecoded> {
        ensure!(self.spend, "not a spend envelope");
        ensure!(self.signed.len() == 104, "TAMPERED: a spend envelope signs exactly 104 bytes");
        let s = &self.signed;
        let u = |i: usize| u64::from_le_bytes(s[i..i + 8].try_into().unwrap());
        Ok(SpendDecoded {
            vault: AccountId::new(s[..32].try_into().unwrap()),
            epoch: u(32),
            nonce: u(40),
            p: u(48),
            to: AccountId::new(s[56..88].try_into().unwrap()),
            amount: u128::from_le_bytes(s[88..].try_into().unwrap()),
        })
    }

    /// `(V, body)` the signatures cover, after the kind's decode checks.
    fn signed_body(&self) -> Result<(AccountId, Vec<u8>)> {
        if self.spend {
            Ok((self.decode_spend()?.vault, self.signed[32..].to_vec()))
        } else {
            let d = self.decode()?;
            Ok((d.vault, d.body.to_vec()))
        }
    }

    /// Adds (or replaces) `sk`'s signature in `role`.
    pub fn sign(&mut self, role: Role, sk: &SigningKey) -> Result<()> {
        ensure!(self.spend == (role == Role::Spend), "role {} does not fit this envelope", role.name());
        let (v, b) = self.signed_body()?;
        let dg = digest(role.tag(), &v, &b);
        let key: [u8; 32] = sk.verifying_key().to_bytes().into();
        let sig = sk.sign_raw(&dg, &[0; 32]).map_err(|e| anyhow!("sign: {e}"))?;
        self.sigs.retain(|s| !(s.key == key && s.role == role));
        self.sigs.push(EnvSig {
            role,
            key,
            sig: sig.to_bytes(),
        });
        Ok(())
    }

    /// Each signature checked against the digest of the signed bytes.
    pub fn verify(&self) -> Result<Vec<(EnvSig, bool)>> {
        let (v, b) = self.signed_body()?;
        Ok(self
            .sigs
            .iter()
            .map(|s| {
                let dg = digest(s.role.tag(), &v, &b);
                let ok = VerifyingKey::from_bytes(&s.key)
                    .ok()
                    .zip(Signature::try_from(s.sig.as_slice()).ok())
                    .is_some_and(|(k, g)| k.verify_prehash(&dg, &g).is_ok())
                    && self.spend == (s.role == Role::Spend);
                (s.clone(), ok)
            })
            .collect())
    }
}

/// Local registry: program account id → name.
pub struct Registry(HashMap<AccountId, String>);

impl Registry {
    /// Built-ins plus an optional JSON file `{"<program account id, base58>": "name"}`.
    pub fn load(path: Option<&std::path::Path>) -> Result<Self> {
        let mut m = HashMap::from([
            (NATIVE_TOKEN_PROGRAM_ID, "native-token".to_string()),
            (msig_host::program_account(), "msig (image v2)".to_string()),
        ]);
        if let Some(p) = path {
            let f: HashMap<String, String> = serde_json::from_slice(&std::fs::read(p)?)?;
            for (k, v) in f {
                m.insert(k.parse().map_err(|e| anyhow!("registry key {k}: {e:?}"))?, v);
            }
        }
        Ok(Self(m))
    }
}

/// Human-readable decoding of the signed bytes; `Ok((text, all_sigs_valid))`.
pub fn render(env: &Envelope, reg: &Registry) -> Result<(String, bool)> {
    let mut o = if env.spend {
        let d = env.decode_spend()?;
        format!(
            "vault        {}\nepoch        {}\nspend nonce  {}\naction       SPEND under the spending limit (no proposal, no time lock)\n  recipient  {}\n  amount     {}\n  period     {} (index; valid only inside that period)\n",
            d.vault, d.epoch, d.nonce, d.to, d.amount, d.p
        )
    } else {
        render_action(&env.decode()?, reg)
    };
    let mut all = true;
    o += "signatures\n";
    for (s, ok) in env.verify()? {
        all &= ok;
        o += &format!(
            "  {:<8} {}  {}\n",
            s.role.name(),
            hex::encode(s.key),
            if ok { "valid" } else { "INVALID" }
        );
    }
    Ok((o, all))
}

fn render_action(d: &Decoded, reg: &Registry) -> String {
    let mut o = format!(
        "vault        {}\nepoch        {}\nnonce        {}\nexpiry       {} (unix ms)\naction hash  {}\n",
        d.vault,
        d.epoch,
        d.nonce,
        d.expiry,
        hex::encode(&d.body[24..])
    );
    o += &format!(
        "approve digest {}\n",
        hex::encode(digest(TAG_APPROVE, &d.vault, &d.body))
    );
    match &d.action {
        Action::Call {
            to,
            data,
            with_authority,
        } => {
            let name = reg.0.get(&to.program_account_id);
            o += &format!(
                "action       CALL\n  program    {}\n  account    {}\n  authority  {}\n",
                match name {
                    Some(n) => format!("{n} ({})", to.program_account_id),
                    None => format!("UNKNOWN program {}", to.program_account_id),
                },
                to.account_id,
                with_authority
            );
            match (to.program_account_id == NATIVE_TOKEN_PROGRAM_ID)
                .then(|| borsh::from_slice::<Native>(data).ok())
                .flatten()
            {
                Some(Native::Transfer { to, amount }) => {
                    o += &format!("  transfer   recipient {to}\n             amount    {amount}\n")
                }
                Some(m) => o += &format!("  native     {m:?}\n"),
                None => o += &format!("  data       {} (not decoded)\n", hex::encode(data)),
            }
        }
        Action::Config(op) => {
            o += &match op {
                ConfigOp::AddMember(m) => format!(
                    "action       CONFIG add member {} perms {:#06b}\n",
                    hex::encode(m.key),
                    m.perms
                ),
                ConfigOp::RemoveMember { key } => {
                    format!("action       CONFIG remove member {}\n", hex::encode(key))
                }
                ConfigOp::SetThreshold(t) => format!("action       CONFIG set threshold {t}\n"),
                ConfigOp::SetTimeLock(ms) => format!("action       CONFIG set time lock {ms} ms\n"),
                ConfigOp::SetLimit(Some(l)) => format!(
                    "action       CONFIG set spending limit {} per {} ms, {} spend signature(s)\n",
                    l.amount, l.period, l.threshold
                ),
                ConfigOp::SetLimit(None) => "action       CONFIG remove the spending limit\n".into(),
            }
        }
        Action::Cancel => o += "action       CANCEL (consumes the nonce, nothing else)\n",
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::Actor;

    fn transfer() -> Action {
        msig_host::native_transfer(AccountId::new([1; 32]), AccountId::new([2; 32]), 300)
    }

    fn signed() -> Envelope {
        let mut e = Envelope::new(&AccountId::new([1; 32]), 3, 4, 99, &transfer());
        e.sign(Role::Propose, &msig_host::dev_key(0)).unwrap();
        e.sign(Role::Approve, &msig_host::dev_key(1)).unwrap();
        e
    }

    #[test]
    fn json_round_trip_and_decode() {
        let e = signed();
        let back = Envelope::from_json(&e.to_json()).unwrap();
        assert_eq!(back, e);
        let d = back.decode().unwrap();
        assert_eq!((d.epoch, d.nonce, d.expiry), (3, 4, 99));
        assert_eq!(d.action, transfer());
        assert!(back.verify().unwrap().iter().all(|(_, ok)| *ok));
        let (txt, ok) = render(&back, &Registry::load(None).unwrap()).unwrap();
        assert!(ok && txt.contains("native-token") && txt.contains("amount    300"), "{txt}");
    }

    #[test]
    fn version_and_garbage_are_refused() {
        let j = signed().to_json().replace("\"version\": 1", "\"version\": 2");
        assert!(Envelope::from_json(&j).unwrap_err().to_string().contains("version 2"));
        assert!(Envelope::from_json("{").is_err());
    }

    #[test]
    fn show_refuses_a_tampered_action() {
        let mut e = signed();
        // flip the low byte of the transfer amount inside the embedded action (last bytes of `signed`)
        let n = e.signed.len();
        let i = n - 16; // amount is the final u128
        e.signed[i] ^= 1;
        let err = render(&e, &Registry::load(None).unwrap()).unwrap_err().to_string();
        assert!(err.contains("TAMPERED"), "{err}");
        assert!(e.sign(Role::Approve, &msig_host::dev_key(2)).is_err());
    }

    #[test]
    fn show_flags_a_signature_over_other_bytes() {
        let mut e = signed();
        e.signed[32] ^= 1; // epoch changes, action hash still matches: sigs no longer cover it
        let (txt, ok) = render(&e, &Registry::load(None).unwrap()).unwrap();
        assert!(!ok && txt.contains("INVALID"), "{txt}");
    }

    #[test]
    fn spend_envelope_signs_the_program_spend_digest() {
        let (v, to) = (AccountId::new([1; 32]), AccountId::new([2; 32]));
        let mut ms = msig_host::initial_state(&msig_host::dev_cfg(3, 2), &[0; 32]);
        (ms.config_epoch, ms.spend.nonce) = (5, 7);
        let mut e = Envelope::new_spend(&v, 5, 7, 9, &to, 100);
        e.sign(Role::Spend, &msig_host::dev_key(1)).unwrap();
        assert!(e.sign(Role::Approve, &msig_host::dev_key(1)).is_err());
        let back = Envelope::from_json(&e.to_json()).unwrap();
        assert_eq!(back, e);
        let d = back.decode_spend().unwrap();
        assert_eq!((d.vault, d.epoch, d.nonce, d.p, d.to, d.amount), (v, 5, 7, 9, to, 100));
        let want = msig_core::spend_digest(&v, &ms, &to, 100, 9);
        let sig = Signature::try_from(back.sigs[0].sig.as_slice()).unwrap();
        msig_host::dev_key(1).verifying_key().verify_prehash(&want, &sig).unwrap();
        let (txt, ok) = render(&back, &Registry::load(None).unwrap()).unwrap();
        assert!(ok && txt.contains("SPEND") && txt.contains("amount     100") && txt.contains("period     9"), "{txt}");
        // the same bytes read as an action envelope are refused
        let mut a = back.clone();
        a.spend = false;
        assert!(render(&a, &Registry::load(None).unwrap()).is_err());
    }

    #[test]
    fn new_config_ops_are_decoded() {
        let v = AccountId::new([1; 32]);
        let show = |op| render(&Envelope::new(&v, 0, 0, 1, &Action::Config(op)), &Registry::load(None).unwrap()).unwrap().0;
        assert!(show(ConfigOp::SetTimeLock(5000)).contains("CONFIG set time lock 5000 ms"));
        let l = msig_core::Limit { amount: 150, period: 86_400_000, threshold: 1 };
        assert!(show(ConfigOp::SetLimit(Some(l))).contains("spending limit 150 per 86400000 ms, 1 spend"));
        assert!(show(ConfigOp::SetLimit(None)).contains("remove the spending limit"));
    }

    #[test]
    fn unknown_program_is_labelled() {
        let a = Action::Call {
            to: Actor::new(AccountId::new([1; 32]), AccountId::new([9; 32])),
            data: vec![1],
            with_authority: false,
        };
        let e = Envelope::new(&AccountId::new([1; 32]), 0, 0, 1, &a);
        assert!(render(&e, &Registry::load(None).unwrap()).unwrap().0.contains("UNKNOWN program"));
    }
}
