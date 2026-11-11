//! Server-side two-phase confirmation. A token is HMAC-SHA256 (per-process random key) over the nonce, expiry,
//! operation, canonical parameters, client user SID, client process id and the digest of the plan shown to the
//! user. Single use (nonce replay set), 120 s lifetime. Any change of parameters, client or plan invalidates it, so
//! the user always confirms exactly what will run.

use crate::identity::ClientIdentity;
use crate::plan::OperationPlan;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use time::{Duration, OffsetDateTime};

pub const LIFETIME: Duration = Duration::seconds(120);
const NONCE: usize = 16;
const MAC: usize = 32;
const TOKEN: usize = NONCE + 8 + MAC;

/// HMAC-SHA256 for keys of at most one block (64 bytes), which is all this service uses.
fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    debug_assert!(key.len() <= 64);
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for (i, k) in key.iter().take(64).enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    for p in parts {
        inner.update(p);
    }
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn plan_digest(plan: &OperationPlan) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(plan).unwrap_or_default()).into()
}

fn b64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=chunk.len() {
            out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let v = |c: u8| match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    };
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= v(*c)? << (18 - 6 * i);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..chunk.len()]);
    }
    Some(out)
}

pub struct TokenService {
    key: [u8; 32],
    used: Mutex<HashMap<[u8; NONCE], i64>>,
}

impl TokenService {
    /// Fails only when the OS random source is unavailable (the broker must not start then).
    pub fn new() -> Result<Self, getrandom::Error> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key)?;
        Ok(Self { key, used: Mutex::new(HashMap::new()) })
    }

    fn mac(
        &self,
        nonce: &[u8],
        expiry: i64,
        operation: &str,
        canonical: &[u8],
        client: &ClientIdentity,
        plan: &OperationPlan,
    ) -> [u8; 32] {
        let field = |data: &[u8]| -> Vec<u8> { [(data.len() as u32).to_le_bytes().as_slice(), data].concat() };
        let sid = client.user_sid.as_deref().unwrap_or("");
        hmac_sha256(
            &self.key,
            &[
                nonce,
                &expiry.to_le_bytes(),
                &field(operation.as_bytes()),
                &field(canonical),
                &field(sid.as_bytes()),
                &u64::from(client.process_id).to_le_bytes(),
                &field(&plan_digest(plan)),
            ],
        )
    }

    pub fn issue(
        &self,
        operation: &str,
        canonical: &[u8],
        client: &ClientIdentity,
        plan: &OperationPlan,
        now: OffsetDateTime,
    ) -> Result<(String, OffsetDateTime), getrandom::Error> {
        let mut nonce = [0u8; NONCE];
        getrandom::fill(&mut nonce)?;
        let expires = now + LIFETIME;
        let expiry = expires.unix_timestamp();
        let mac = self.mac(&nonce, expiry, operation, canonical, client, plan);
        Ok((b64url(&[nonce.as_slice(), &expiry.to_le_bytes(), &mac].concat()), expires))
    }

    /// Verifies and consumes the token; a second use fails.
    pub fn consume(
        &self,
        token: &str,
        operation: &str,
        canonical: &[u8],
        client: &ClientIdentity,
        plan: &OperationPlan,
        now: OffsetDateTime,
    ) -> bool {
        let Some(raw) = (token.len() <= crate::envelope::MAX_TOKEN_LENGTH)
            .then(|| b64url_decode(token))
            .flatten()
            .filter(|r| r.len() == TOKEN)
        else {
            return false;
        };
        let mut nonce = [0u8; NONCE];
        nonce.copy_from_slice(&raw[..NONCE]);
        let mut expiry_bytes = [0u8; 8];
        expiry_bytes.copy_from_slice(&raw[NONCE..NONCE + 8]);
        let expiry = i64::from_le_bytes(expiry_bytes);
        let now_s = now.unix_timestamp();
        if expiry < now_s || expiry > now_s + 2 * LIFETIME.whole_seconds() {
            return false;
        }
        if !constant_time_eq(&self.mac(&nonce, expiry, operation, canonical, client, plan), &raw[NONCE + 8..]) {
            return false;
        }
        let Ok(mut used) = self.used.lock() else { return false };
        if used.len() >= 256 {
            used.retain(|_, e| *e >= now_s);
        }
        used.insert(nonce, expiry).is_none()
    }
}

impl Drop for TokenService {
    fn drop(&mut self) {
        self.key.iter_mut().for_each(|b| *b = 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::{APP, client};
    use bitbybit_core::{RiskLevel, text};
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-11 10:00 UTC);

    fn plan(step: &str) -> OperationPlan {
        OperationPlan::new(
            text!("x.summary", "Summary"),
            vec![text!("x.step", "{0}", step.to_owned())],
            RiskLevel::Medium,
        )
    }

    #[test]
    fn token_is_single_use_and_bound_to_parameters_client_and_plan() {
        let s = TokenService::new().unwrap();
        let c = client(APP, false);
        let (t, expires) = s.issue("network.startSharing", b"{\"a\":1}", &c, &plan("one"), NOW).unwrap();
        assert_eq!(expires, NOW + LIFETIME);
        assert!(
            t.len() <= crate::envelope::MAX_TOKEN_LENGTH
                && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert!(!s.consume(&t, "network.startSharing", b"{\"a\":2}", &c, &plan("one"), NOW), "parameters changed");
        assert!(
            !s.consume(
                &t,
                "network.startSharing",
                b"{\"a\":1}",
                &ClientIdentity { process_id: 1, ..c.clone() },
                &plan("one"),
                NOW
            ),
            "client changed"
        );
        assert!(!s.consume(&t, "network.startSharing", b"{\"a\":1}", &c, &plan("two"), NOW), "plan changed");
        assert!(!s.consume(&t, "network.stopSharing", b"{\"a\":1}", &c, &plan("one"), NOW), "operation changed");
        assert!(s.consume(&t, "network.startSharing", b"{\"a\":1}", &c, &plan("one"), NOW));
        assert!(!s.consume(&t, "network.startSharing", b"{\"a\":1}", &c, &plan("one"), NOW), "replay");
    }

    #[test]
    fn token_expires_and_garbage_is_rejected() {
        let s = TokenService::new().unwrap();
        let c = client(APP, false);
        let (t, _) = s.issue("a.b", b"{}", &c, &plan("x"), NOW).unwrap();
        assert!(!s.consume(&t, "a.b", b"{}", &c, &plan("x"), NOW + Duration::seconds(121)));
        for garbage in ["", "abc", "!!!!", &"A".repeat(200), &"A".repeat(75)] {
            assert!(!s.consume(garbage, "a.b", b"{}", &c, &plan("x"), NOW));
        }
    }

    #[test]
    fn tokens_from_another_broker_instance_are_rejected() {
        let (a, b) = (TokenService::new().unwrap(), TokenService::new().unwrap());
        let c = client(APP, false);
        let (t, _) = a.issue("a.b", b"{}", &c, &plan("x"), NOW).unwrap();
        assert!(!b.consume(&t, "a.b", b"{}", &c, &plan("x"), NOW));
    }

    #[test]
    fn hmac_matches_rfc4231_test_case_2() {
        let hex: String =
            hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"]).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(b64url_decode(&b64url(b"\x00\x01\x02\xfe\xff")).as_deref(), Some(&b"\x00\x01\x02\xfe\xff"[..]));
    }
}
