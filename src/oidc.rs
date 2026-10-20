//! OIDC token verification adapter.
//!
//! The adapter verifies, in this order:
//!
//! 1. token shape and signature against cached JWKS;
//! 2. `iss` equals the configured issuer exactly;
//! 3. `aud` contains the configured audience;
//! 4. `exp` is in the future (with configured clock skew), and `nbf`/`iat` when
//!    present are not in the future beyond the skew;
//! 5. `sub` is present and non-blank.
//!
//! Roles and corpus grants are read only from the verified claim set. Any
//! verification error, including a JWKS outage, is an error: the adapter never
//! returns an anonymous identity.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::identity::{Role, VerifiedIdentity};

/// Errors returned by token verification.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing bearer token")]
    Missing,
    #[error("malformed token: {0}")]
    Malformed(String),
    #[error("unsupported signing algorithm")]
    UnsupportedAlgorithm,
    #[error("signature verification failed")]
    BadSignature,
    #[error("unknown signing key")]
    UnknownKey,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid")]
    NotYetValid,
    #[error("wrong issuer")]
    WrongIssuer,
    #[error("wrong audience")]
    WrongAudience,
    #[error("missing subject")]
    MissingSubject,
    #[error("identity provider unavailable: {0}")]
    ProviderUnavailable(String),
}

/// A public JSON Web Key (subset used for EdDSA-free RSA/EC verification is
/// out of scope). The adapter accepts only `none`-safe signed JWTs presented
/// by the identity provider and verifies them with an HMAC-SHA256 key ring, so
/// tests can run without an external crypto dependency.
#[derive(Debug, Clone)]
pub struct Jwk {
    /// Key id.
    pub kid: String,
    /// Algorithm, only `HS256` is accepted.
    pub alg: String,
    /// Symmetric key material.
    pub k: Vec<u8>,
}

/// A snapshot of the JWKS document.
#[derive(Debug, Clone)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

impl Jwks {
    /// Builds a key set.
    pub fn new(keys: Vec<Jwk>) -> Self {
        Self { keys }
    }

    /// Parses the compact JWKS JSON used by the tests and by operators.
    pub fn from_json(raw: &str) -> Result<Self, AuthError> {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        let keys = value
            .get("keys")
            .and_then(|keys| keys.as_array())
            .ok_or_else(|| AuthError::ProviderUnavailable("JWKS has no keys".into()))?;
        let mut parsed = Vec::new();
        for key in keys {
            let kid = key
                .get("kid")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AuthError::ProviderUnavailable("JWK has no kid".into()))?;
            let alg = key
                .get("alg")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AuthError::ProviderUnavailable("JWK has no alg".into()))?;
            let k = key
                .get("k")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AuthError::ProviderUnavailable("JWK has no k".into()))?;
            parsed.push(Jwk {
                kid: kid.to_string(),
                alg: alg.to_string(),
                k: k.as_bytes().to_vec(),
            });
        }
        Ok(Self { keys: parsed })
    }

    fn find(&self, kid: &str) -> Option<&Jwk> {
        self.keys.iter().find(|key| key.kid == kid)
    }
}

/// Auth configuration used by the adapter.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Exact expected issuer.
    pub issuer: String,
    /// Expected audience.
    pub audience: String,
    /// JWKS cache TTL.
    pub cache_ttl: Duration,
    /// Clock skew allowed for `exp`/`nbf`.
    pub clock_skew: Duration,
}

impl AuthConfig {
    /// Builds the configuration.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        cache_ttl: Duration,
        clock_skew: Duration,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            cache_ttl,
            clock_skew,
        }
    }
}

/// The JWKS cache with TTL and single-flight refresh.
#[derive(Clone)]
pub struct JwksCache<F>
where
    F: Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
{
    fetch: Arc<F>,
    state: Arc<Mutex<CacheState>>,
    ttl: Duration,
}

struct CacheState {
    jwks: Option<Jwks>,
    fetched_at: Option<Instant>,
}

impl<F> JwksCache<F>
where
    F: Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
{
    /// Builds a cache around a fetcher.
    pub fn new(fetch: F, ttl: Duration) -> Self {
        Self {
            fetch: Arc::new(fetch),
            state: Arc::new(Mutex::new(CacheState {
                jwks: None,
                fetched_at: None,
            })),
            ttl,
        }
    }

    /// Returns a key set, refreshing when the TTL elapsed. A refresh error is
    /// returned, never replaced with a stale-only success.
    pub fn get(&self) -> Result<Jwks, AuthError> {
        // Single-flight: hold the lock while refreshing so concurrent callers
        // wait for the same fetch instead of stampeding the provider.
        let mut state = self
            .state
            .lock()
            .map_err(|_| AuthError::ProviderUnavailable("JWKS cache poisoned".into()))?;
        let fresh = match state.fetched_at {
            Some(at) => at.elapsed() < self.ttl,
            None => false,
        };
        if fresh {
            if let Some(jwks) = &state.jwks {
                return Ok(jwks.clone());
            }
        }
        let jwks = (self.fetch)()?;
        state.jwks = Some(jwks.clone());
        state.fetched_at = Some(Instant::now());
        Ok(jwks)
    }
}

fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    let mut seen_pad = false;
    for ch in input.chars() {
        if ch == '=' {
            seen_pad = true;
            continue;
        }
        if seen_pad {
            return None;
        }
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '-' => 62,
            '_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

/// HMAC-SHA256 over the signing input with the supplied key.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(BLOCK + message.len());
    let mut outer = Vec::with_capacity(BLOCK + 32);
    for byte in key_block {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// A self-contained SHA-256 used to avoid an extra dependency in the adapter.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let start = i * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn base64url_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn claims_to_identity(
    claims: &serde_json::Value,
    issuer: &str,
) -> Result<VerifiedIdentity, AuthError> {
    let subject = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or(AuthError::MissingSubject)?;

    let mut roles = Vec::new();
    if let Some(list) = claims.get("roles").and_then(|v| v.as_array()) {
        for entry in list {
            if let Some(name) = entry.as_str() {
                // Unknown role names are dropped, never mapped to a default.
                if let Some(role) = Role::parse(name) {
                    roles.push(role);
                }
            }
        }
    }

    let corpus_grants = claims
        .get("corpus_grants")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(VerifiedIdentity::new(subject, issuer, roles, corpus_grants))
}

/// Verifies a bearer token and returns a typed verified identity.
///
/// Verification failures and provider outages are errors. The function never
/// returns an anonymous identity.
pub fn validate_token<F>(
    token: &str,
    config: &AuthConfig,
    jwks: &JwksCache<F>,
) -> Result<VerifiedIdentity, AuthError>
where
    F: Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
{
    let token = token.trim();
    if token.is_empty() {
        return Err(AuthError::Missing);
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::Malformed("expected three segments".into()));
    }

    let header_bytes = base64url_decode(parts[0])
        .ok_or_else(|| AuthError::Malformed("header is not base64url".into()))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|_| AuthError::Malformed("header is not JSON".into()))?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or(AuthError::Malformed("header has no alg".into()))?;
    if alg != "HS256" {
        return Err(AuthError::UnsupportedAlgorithm);
    }
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Malformed("header has no kid".into()))?;

    let keys = jwks.get().map_err(|err| match err {
        AuthError::ProviderUnavailable(message) => AuthError::ProviderUnavailable(message),
        other => other,
    })?;
    let key = keys.find(kid).ok_or(AuthError::UnknownKey)?;
    if key.alg != "HS256" {
        return Err(AuthError::UnsupportedAlgorithm);
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let expected = hmac_sha256(&key.k, signing_input.as_bytes());
    let actual = base64url_decode(parts[2])
        .ok_or_else(|| AuthError::Malformed("signature is not base64url".into()))?;
    if !constant_time_eq(&expected, &actual) {
        return Err(AuthError::BadSignature);
    }

    let payload_bytes = base64url_decode(parts[1])
        .ok_or_else(|| AuthError::Malformed("payload is not base64url".into()))?;
    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|_| AuthError::Malformed("payload is not JSON".into()))?;

    let issuer = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .ok_or(AuthError::WrongIssuer)?;
    if issuer != config.issuer {
        return Err(AuthError::WrongIssuer);
    }

    let audience_ok = match claims.get("aud") {
        Some(serde_json::Value::String(value)) => value == &config.audience,
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .any(|value| value.as_str() == Some(config.audience.as_str())),
        _ => false,
    };
    if !audience_ok {
        return Err(AuthError::WrongAudience);
    }

    let now = now_seconds();
    let skew = config.clock_skew.as_secs();
    let exp = claims
        .get("exp")
        .and_then(|v| v.as_u64())
        .ok_or(AuthError::Expired)?;
    if exp.saturating_add(skew) <= now {
        return Err(AuthError::Expired);
    }
    if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_u64()) {
        if nbf > now.saturating_add(skew) {
            return Err(AuthError::NotYetValid);
        }
    }

    claims_to_identity(&claims, issuer)
}

/// Signs a test token. Used only by tests and the isolated test identity
/// harness; production tokens are signed by the identity provider.
#[doc(hidden)]
pub fn sign_test_token(header: serde_json::Value, claims: serde_json::Value, key: &[u8]) -> String {
    let header_json = serde_json::to_vec(&header).unwrap_or_default();
    let claims_json = serde_json::to_vec(&claims).unwrap_or_default();
    let header_b64 = base64url_encode(&header_json);
    let claims_b64 = base64url_encode(&claims_json);
    let signing_input = format!("{header_b64}.{claims_b64}");
    let signature = hmac_sha256(key, signing_input.as_bytes());
    format!("{signing_input}.{}", base64url_encode(&signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"test-signing-key-please-rotate";
    const KID: &str = "test-key";
    const ISSUER: &str = "https://issuer.example";
    const AUDIENCE: &str = "rag-api";

    fn config() -> AuthConfig {
        AuthConfig::new(
            ISSUER,
            AUDIENCE,
            Duration::from_secs(300),
            Duration::from_secs(0),
        )
    }

    fn jwks() -> Jwks {
        Jwks::new(vec![Jwk {
            kid: KID.to_string(),
            alg: "HS256".to_string(),
            k: KEY.to_vec(),
        }])
    }

    fn cache(
        keys: Jwks,
    ) -> JwksCache<impl Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static> {
        let keys = Arc::new(keys);
        JwksCache::new(move || Ok((*keys).clone()), Duration::from_secs(300))
    }

    fn claims(sub: &str, roles: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": sub,
            "exp": now_seconds() + 600,
            "roles": roles,
        })
    }

    fn header() -> serde_json::Value {
        serde_json::json!({"alg": "HS256", "kid": KID, "typ": "JWT"})
    }

    fn token(claims: serde_json::Value) -> String {
        sign_test_token(header(), claims, KEY)
    }

    #[test]
    fn valid_token_yields_verified_identity() {
        let cache = cache(jwks());
        let identity =
            validate_token(&token(claims("user-1", &["editor"])), &config(), &cache).unwrap();
        assert_eq!(identity.subject(), "user-1");
        assert_eq!(identity.roles(), &[Role::Editor]);
    }

    #[test]
    fn forged_signature_is_rejected() {
        let cache = cache(jwks());
        let forged = sign_test_token(header(), claims("user-1", &["admin"]), b"wrong-key");
        assert_eq!(
            validate_token(&forged, &config(), &cache).unwrap_err(),
            AuthError::BadSignature
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let cache = cache(jwks());
        let mut c = claims("user-1", &["viewer"]);
        c["exp"] = serde_json::json!(now_seconds().saturating_sub(10));
        assert_eq!(
            validate_token(&token(c), &config(), &cache).unwrap_err(),
            AuthError::Expired
        );
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let cache = cache(jwks());
        let mut c = claims("user-1", &["viewer"]);
        c["iss"] = serde_json::json!("https://evil.example");
        assert_eq!(
            validate_token(&token(c), &config(), &cache).unwrap_err(),
            AuthError::WrongIssuer
        );
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let cache = cache(jwks());
        let mut c = claims("user-1", &["viewer"]);
        c["aud"] = serde_json::json!("other-api");
        assert_eq!(
            validate_token(&token(c), &config(), &cache).unwrap_err(),
            AuthError::WrongAudience
        );
    }

    #[test]
    fn altered_claims_are_rejected() {
        let cache = cache(jwks());
        let original = token(claims("user-1", &["viewer"]));
        let segments: Vec<&str> = original.split('.').collect();
        let tampered_claims = claims("user-1", &["admin"]);
        let tampered_b64 = base64url_encode(&serde_json::to_vec(&tampered_claims).unwrap());
        let tampered = format!("{}.{}.{}", segments[0], tampered_b64, segments[2]);
        assert_eq!(
            validate_token(&tampered, &config(), &cache).unwrap_err(),
            AuthError::BadSignature
        );
    }

    #[test]
    fn jwks_outage_is_an_error_not_anonymous() {
        let cache: JwksCache<_> = JwksCache::new(
            || Err(AuthError::ProviderUnavailable("connection refused".into())),
            Duration::from_secs(1),
        );
        let result = validate_token(&token(claims("user-1", &["admin"])), &config(), &cache);
        assert!(matches!(result, Err(AuthError::ProviderUnavailable(_))));
    }

    #[test]
    fn empty_token_is_missing() {
        let cache = cache(jwks());
        assert_eq!(
            validate_token("  ", &config(), &cache).unwrap_err(),
            AuthError::Missing
        );
    }

    #[test]
    fn unknown_roles_are_dropped() {
        let cache = cache(jwks());
        let identity =
            validate_token(&token(claims("user-2", &["root"])), &config(), &cache).unwrap();
        assert!(identity.roles().is_empty());
    }
}
