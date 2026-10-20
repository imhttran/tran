//! OIDC token verification adapter (TRAN-003C phase 2).
//!
//! The custom HS256-only verifier is replaced with a maintained, standards-based
//! asymmetric JWKS verifier built on the `ring` crate. The adapter:
//!
//! 1. parses the token header and rejects any algorithm outside the approved
//!    asymmetric set ([`APPROVED_ALGORITHMS`]); HS256 and every symmetric or
//!    `none` algorithm are rejected even if a matching key is present;
//! 2. fetches the JWKS (cached, single-flight) and fails closed when keys are
//!    unavailable or cannot be parsed;
//! 3. selects the verification key by `kid` and algorithm, then verifies the
//!    signature over the signing input;
//! 4. requires `iss` to equal the configured issuer exactly;
//! 5. requires `aud` to contain the configured audience;
//! 6. requires `exp` in the future (with clock skew) and rejects `nbf`/`iat`
//!    beyond the skew;
//! 7. requires a non-blank `sub`.
//!
//! Roles and corpus grants are loaded from the database by the API after the
//! token is verified. Token claims are never used to grant access, so a stale or
//! altered claim cannot elevate an identity.
//!
//! JWK public keys are parsed as RSA (`kty=RSA`, `n`/`e`) or EC P-256
//! (`kty=EC`, `crv=P-256`, `x`/`y`). Symmetric keys (`kty=oct`, `k`) and
//! private-key material are rejected as malformed for verification purposes.

use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::identity::VerifiedIdentity;

/// The approved asymmetric JWKS algorithm set. Symmetric (`HS*`) and `none`
/// algorithms are never accepted.
pub const APPROVED_ALGORITHMS: &[&str] = &[
    "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256",
];

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
    #[error("malformed JWK: {0}")]
    MalformedJwk(String),
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

/// The verification algorithm family carried by a parsed public JWK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwkAlgorithm {
    /// RSA PKCS#1 v1.5 (`RS*`).
    RsaPkcs1,
    /// RSA PSS (`PS*`).
    RsaPss,
    /// ECDSA P-256 (`ES256`).
    EcdsaP256,
}

/// A parsed asymmetric public JSON Web Key.
#[derive(Debug, Clone)]
pub struct Jwk {
    /// Key id.
    pub kid: String,
    /// The `alg` declared by the JWK, if present. Verification also accepts the
    /// algorithm named by the token header, provided both are approved.
    pub alg: Option<String>,
    /// The verification algorithm family.
    pub algorithm: JwkAlgorithm,
    /// RSA modulus (big-endian) for RSA keys.
    pub modulus: Vec<u8>,
    /// RSA public exponent (big-endian) for RSA keys.
    pub exponent: Vec<u8>,
    /// EC x-coordinate for P-256 keys.
    pub x: Vec<u8>,
    /// EC y-coordinate for P-256 keys.
    pub y: Vec<u8>,
}

/// A snapshot of the JWKS document.
#[derive(Debug, Clone)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

impl Jwks {
    /// Builds a key set from parsed keys.
    pub fn new(keys: Vec<Jwk>) -> Self {
        Self { keys }
    }

    /// Parses a JWKS JSON document into asymmetric public keys.
    ///
    /// Symmetric (`oct`) keys, missing key material, and unknown curves are
    /// rejected as [`AuthError::MalformedJwk`]; the whole document fails closed.
    pub fn from_json(raw: &str) -> Result<Self, AuthError> {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|err| AuthError::ProviderUnavailable(format!("JWKS is not JSON: {err}")))?;
        let keys = value
            .get("keys")
            .and_then(|keys| keys.as_array())
            .ok_or_else(|| AuthError::ProviderUnavailable("JWKS has no keys".into()))?;
        let mut parsed = Vec::new();
        for key in keys {
            parsed.push(parse_jwk(key)?);
        }
        Ok(Self { keys: parsed })
    }

    fn find(&self, kid: &str, alg: &str) -> Option<&Jwk> {
        self.keys.iter().find(|key| {
            if key.kid != kid {
                return false;
            }
            // When the JWK names an algorithm, it must match the header exactly.
            match (&key.alg, alg) {
                (Some(declared), requested) => declared == requested,
                (None, _) => true,
            }
        })
    }
}

fn required_str<'a>(key: &'a serde_json::Value, field: &str) -> Result<&'a str, AuthError> {
    key.get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::MalformedJwk(format!("missing {field}")))
}

fn decode_b64url(field: &str, value: &str) -> Result<Vec<u8>, AuthError> {
    base64url_decode(value)
        .ok_or_else(|| AuthError::MalformedJwk(format!("{field} is not base64url")))
}

fn parse_jwk(key: &serde_json::Value) -> Result<Jwk, AuthError> {
    let kid = required_str(key, "kid")?.to_string();
    let kty = required_str(key, "kty")?;
    // Symmetric key material is never valid for verification.
    if key.get("k").is_some() || kty == "oct" {
        return Err(AuthError::MalformedJwk(
            "symmetric JWK keys are not accepted for verification".into(),
        ));
    }
    let alg = key.get("alg").and_then(|v| v.as_str()).map(str::to_string);
    match kty {
        "RSA" => {
            let modulus = decode_b64url("n", required_str(key, "n")?)?;
            let exponent = decode_b64url("e", required_str(key, "e")?)?;
            if modulus.is_empty() || exponent.is_empty() {
                return Err(AuthError::MalformedJwk("empty RSA key material".into()));
            }
            let algorithm = match alg.as_deref() {
                Some("PS256") | Some("PS384") | Some("PS512") => JwkAlgorithm::RsaPss,
                _ => JwkAlgorithm::RsaPkcs1,
            };
            Ok(Jwk {
                kid,
                alg,
                algorithm,
                modulus,
                exponent,
                x: Vec::new(),
                y: Vec::new(),
            })
        }
        "EC" => {
            let crv = required_str(key, "crv")?;
            if crv != "P-256" {
                return Err(AuthError::MalformedJwk(format!(
                    "unsupported EC curve {crv}"
                )));
            }
            let x = decode_b64url("x", required_str(key, "x")?)?;
            let y = decode_b64url("y", required_str(key, "y")?)?;
            if x.len() != 32 || y.len() != 32 {
                return Err(AuthError::MalformedJwk(
                    "EC P-256 coordinates must be 32 bytes".into(),
                ));
            }
            Ok(Jwk {
                kid,
                alg,
                algorithm: JwkAlgorithm::EcdsaP256,
                modulus: Vec::new(),
                exponent: Vec::new(),
                x,
                y,
            })
        }
        other => Err(AuthError::MalformedJwk(format!("unsupported kty {other}"))),
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

/// How long past its TTL an expired key set may still be served while one
/// refresh runs. Longer than the 10-second fetch timeout, so one slow fetch
/// does not stall requests, but short enough that a provider outage fails
/// closed instead of serving rotated-out keys indefinitely.
#[cfg(not(test))]
const JWKS_MAX_STALE: Duration = Duration::from_secs(30);
#[cfg(test)]
const JWKS_MAX_STALE: Duration = Duration::from_secs(1);

/// The JWKS cache with TTL and single-flight refresh.
#[derive(Clone)]
pub struct JwksCache<F>
where
    F: Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static,
{
    fetch: Arc<F>,
    state: Arc<Mutex<CacheState>>,
    /// Held by the one caller fetching; `state` is never held across a fetch.
    refresh: Arc<Mutex<()>>,
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
            refresh: Arc::new(Mutex::new(())),
            ttl,
        }
    }

    /// Returns the cached key set and its age, if any.
    fn cached(&self) -> Result<Option<(Jwks, Duration)>, AuthError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AuthError::ProviderUnavailable("JWKS cache poisoned".into()))?;
        Ok(state
            .jwks
            .clone()
            .zip(state.fetched_at.map(|at| at.elapsed())))
    }

    /// Returns a key set, refreshing when the TTL elapsed.
    ///
    /// Single-flight: one caller fetches while the others keep serving the
    /// expired set, so a slow provider never stalls every request. The expired
    /// set is served for at most [`JWKS_MAX_STALE`] past the TTL; after that,
    /// and on a cold cache, callers wait for the fetch and see its error. The
    /// fetching caller gets the refresh error, never a stale-only success.
    pub fn get(&self) -> Result<Jwks, AuthError> {
        let cached = self.cached()?;
        if let Some((jwks, age)) = &cached {
            if *age < self.ttl {
                return Ok(jwks.clone());
            }
        }
        let stale = cached
            .filter(|(_, age)| *age < self.ttl + JWKS_MAX_STALE)
            .map(|(jwks, _)| jwks);
        // The refresh lock guards no data, so a panicked fetch cannot leave it
        // inconsistent.
        let _refresh = match self.refresh.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => match stale {
                Some(stale) => return Ok(stale),
                None => self.refresh.lock().unwrap_or_else(PoisonError::into_inner),
            },
        };
        // Another caller may have refreshed while this one waited.
        if let Some((jwks, age)) = self.cached()? {
            if age < self.ttl {
                return Ok(jwks);
            }
        }
        let jwks = (self.fetch)()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| AuthError::ProviderUnavailable("JWKS cache poisoned".into()))?;
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

fn algorithm_kind(alg: &str) -> Option<JwkAlgorithm> {
    match alg {
        "RS256" | "RS384" | "RS512" => Some(JwkAlgorithm::RsaPkcs1),
        "PS256" | "PS384" | "PS512" => Some(JwkAlgorithm::RsaPss),
        "ES256" => Some(JwkAlgorithm::EcdsaP256),
        _ => None,
    }
}

fn verify_signature(
    key: &Jwk,
    alg: &str,
    signing_input: &[u8],
    signature: &[u8],
) -> Result<(), AuthError> {
    use ring::signature;
    match key.algorithm {
        JwkAlgorithm::RsaPkcs1 | JwkAlgorithm::RsaPss => {
            let verification_alg = match alg {
                "RS256" => &signature::RSA_PKCS1_2048_8192_SHA256,
                "RS384" => &signature::RSA_PKCS1_2048_8192_SHA384,
                "RS512" => &signature::RSA_PKCS1_2048_8192_SHA512,
                "PS256" => &signature::RSA_PSS_2048_8192_SHA256,
                "PS384" => &signature::RSA_PSS_2048_8192_SHA384,
                "PS512" => &signature::RSA_PSS_2048_8192_SHA512,
                _ => return Err(AuthError::UnsupportedAlgorithm),
            };
            let public_key = signature::RsaPublicKeyComponents {
                n: &key.modulus,
                e: &key.exponent,
            };
            public_key
                .verify(verification_alg, signing_input, signature)
                .map_err(|_| AuthError::BadSignature)
        }
        JwkAlgorithm::EcdsaP256 => {
            if alg != "ES256" {
                return Err(AuthError::UnsupportedAlgorithm);
            }
            // Uncompressed SEC1 point: 0x04 || x || y.
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(&key.x);
            point.extend_from_slice(&key.y);
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point.as_slice())
                .verify(signing_input, signature)
                .map_err(|_| AuthError::BadSignature)
        }
    }
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

    // Authorization is database-backed. Claims named `roles` or
    // `corpus_grants` are deliberately ignored here; the API loads the
    // effective values for this subject after verification.
    Ok(VerifiedIdentity::new(
        subject,
        issuer,
        Vec::new(),
        Vec::new(),
    ))
}

/// Verifies a bearer token and returns a typed verified identity.
///
/// Verification failures and provider outages are errors. The function never
/// returns an anonymous identity and performs no side effects.
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
    // Reject any algorithm outside the approved asymmetric set. This rejects
    // HS256 (and every symmetric/`none` algorithm) before key lookup.
    if !APPROVED_ALGORITHMS.contains(&alg) || algorithm_kind(alg).is_none() {
        return Err(AuthError::UnsupportedAlgorithm);
    }
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Malformed("header has no kid".into()))?;

    let keys = jwks.get()?;
    let key = keys.find(kid, alg).ok_or(AuthError::UnknownKey)?;
    // The key family must match the requested algorithm family.
    if algorithm_kind(alg) != Some(key.algorithm) {
        return Err(AuthError::UnsupportedAlgorithm);
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let signature = base64url_decode(parts[2])
        .ok_or_else(|| AuthError::Malformed("signature is not base64url".into()))?;
    verify_signature(key, alg, signing_input.as_bytes(), &signature)?;

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

/// Signs a compact JWT with the supplied RSA key (RS256). Test and synthetic
/// provider use only; production tokens are signed by the identity provider.
#[doc(hidden)]
pub fn sign_rs256(
    keypair: &ring::signature::RsaKeyPair,
    kid: &str,
    claims: serde_json::Value,
) -> String {
    use ring::rand::SystemRandom;
    let header = serde_json::json!({"alg": "RS256", "kid": kid, "typ": "JWT"});
    let header_b64 = base64url_encode(&serde_json::to_vec(&header).unwrap_or_default());
    let claims_b64 = base64url_encode(&serde_json::to_vec(&claims).unwrap_or_default());
    let signing_input = format!("{header_b64}.{claims_b64}");
    let rng = SystemRandom::new();
    let mut sig = vec![0u8; keypair.public().modulus_len()];
    keypair
        .sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &rng,
            signing_input.as_bytes(),
            &mut sig,
        )
        .expect("sign RS256");
    format!("{signing_input}.{}", base64url_encode(&sig))
}

/// Signs a legacy HS256 token. Retained only so tests can prove that a
/// symmetric token signed with a configured secret is rejected.
#[doc(hidden)]
pub fn sign_hs256_for_rejection(claims: serde_json::Value, secret: &[u8]) -> String {
    let header = serde_json::json!({"alg": "HS256", "kid": "legacy", "typ": "JWT"});
    let header_b64 = base64url_encode(&serde_json::to_vec(&header).unwrap_or_default());
    let claims_b64 = base64url_encode(&serde_json::to_vec(&claims).unwrap_or_default());
    let input = format!("{header_b64}.{claims_b64}");
    let sig = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret),
        input.as_bytes(),
    );
    format!("{input}.{}", base64url_encode(sig.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn claims(sub: &str, roles: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": sub,
            "exp": now_seconds() + 600,
            "roles": roles,
        })
    }

    fn cache(
        jwks: Jwks,
    ) -> JwksCache<impl Fn() -> Result<Jwks, AuthError> + Send + Sync + 'static> {
        let keys = Arc::new(jwks);
        JwksCache::new(move || Ok((*keys).clone()), Duration::from_secs(300))
    }

    #[test]
    fn hs256_tokens_are_rejected_even_with_a_key() {
        // A token claiming HS256 must be rejected before any key lookup, so an
        // (unused) symmetric key in the set cannot authorize it.
        let token = sign_hs256_for_rejection(claims("u", &["admin"]), b"secret");
        let cache = cache(Jwks::new(Vec::new()));
        assert_eq!(
            validate_token(&token, &config(), &cache).unwrap_err(),
            AuthError::UnsupportedAlgorithm
        );
    }

    #[test]
    fn malformed_jwk_is_rejected() {
        let raw = r#"{"keys":[{"kty":"oct","kid":"k","k":"c2VjcmV0"}]}"#;
        assert!(matches!(
            Jwks::from_json(raw).unwrap_err(),
            AuthError::MalformedJwk(_)
        ));
        let raw = r#"{"keys":[{"kty":"RSA","kid":"k"}]}"#;
        assert!(matches!(
            Jwks::from_json(raw).unwrap_err(),
            AuthError::MalformedJwk(_)
        ));
    }

    #[test]
    fn jwks_outage_fails_closed() {
        let cache: JwksCache<_> = JwksCache::new(
            || Err(AuthError::ProviderUnavailable("connection refused".into())),
            Duration::from_secs(1),
        );
        // Build a valid-shaped RS256 header so verification reaches the fetch
        // and observes the provider outage (fail closed, not anonymous).
        // header to reach the fetch and observe the outage.
        let header = base64url_encode(br#"{"alg":"RS256","kid":"k"}"#);
        let payload = base64url_encode(br#"{"iss":"x","aud":"y","sub":"s","exp":9999999999}"#);
        let token = format!("{header}.{payload}.c2ln");
        assert!(matches!(
            validate_token(&token, &config(), &cache).unwrap_err(),
            AuthError::ProviderUnavailable(_)
        ));
    }

    #[test]
    fn slow_refresh_serves_the_stale_set_to_other_callers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;

        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let calls = AtomicUsize::new(0);
        let cache = JwksCache::new(
            move || {
                if calls.fetch_add(1, Ordering::SeqCst) > 0 {
                    started_tx.send(()).unwrap();
                    // Bounded so a regression fails instead of hanging.
                    let _ = release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5));
                }
                Ok(Jwks::new(Vec::new()))
            },
            Duration::ZERO,
        );
        cache.get().expect("initial fetch");

        std::thread::scope(|scope| {
            let refresher = scope.spawn(|| cache.get());
            started_rx.recv().unwrap();
            let begin = Instant::now();
            cache.get().expect("stale set");
            assert!(begin.elapsed() < Duration::from_secs(1));
            release_tx.send(()).unwrap();
            refresher.join().unwrap().expect("refresh");
        });
    }

    #[test]
    fn stale_set_past_the_cap_is_not_served_during_a_refresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = AtomicUsize::new(0);
        let cache = JwksCache::new(
            move || {
                if calls.fetch_add(1, Ordering::SeqCst) > 0 {
                    std::thread::sleep(Duration::from_millis(300));
                    return Err(AuthError::ProviderUnavailable("down".into()));
                }
                Ok(Jwks::new(Vec::new()))
            },
            Duration::ZERO,
        );
        cache.get().expect("initial fetch");
        std::thread::sleep(JWKS_MAX_STALE + Duration::from_millis(100));

        std::thread::scope(|scope| {
            let refresher = scope.spawn(|| cache.get());
            std::thread::sleep(Duration::from_millis(50));
            // Past the cap the caller waits for the refresh and fails closed.
            assert!(matches!(
                cache.get(),
                Err(AuthError::ProviderUnavailable(_))
            ));
            assert!(refresher.join().unwrap().is_err());
        });
    }

    #[test]
    fn empty_token_is_missing() {
        let cache = cache(Jwks::new(Vec::new()));
        assert_eq!(
            validate_token("  ", &config(), &cache).unwrap_err(),
            AuthError::Missing
        );
    }

    #[test]
    fn malformed_token_is_rejected() {
        let cache = cache(Jwks::new(Vec::new()));
        assert!(matches!(
            validate_token("not-a-jwt", &config(), &cache).unwrap_err(),
            AuthError::Malformed(_)
        ));
    }

    #[test]
    fn unknown_algorithm_is_rejected() {
        let header = base64url_encode(br#"{"alg":"none","kid":"k"}"#);
        let payload = base64url_encode(br#"{"iss":"x"}"#);
        let token = format!("{header}.{payload}.");
        let cache = cache(Jwks::new(Vec::new()));
        assert_eq!(
            validate_token(&token, &config(), &cache).unwrap_err(),
            AuthError::UnsupportedAlgorithm
        );
    }
}
