use argon2::{
    Algorithm as Argon2Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier,
    Version, password_hash::SaltString,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{SigningKey, pkcs8::DecodePrivateKey};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use uuid::Uuid;

const AUTH_RATE_LIMIT: u32 = 120;
const AUTH_RATE_PERIOD: Duration = Duration::from_mins(1);
const AUTH_RATE_MAX_KEYS: usize = 10_000;
const REAUTH_NONCE_TTL: Duration = Duration::from_mins(5);
const DEFAULT_ARGON2_MEMORY_KIB: u32 = 19_456;
const DEFAULT_ARGON2_TIME_COST: u32 = 2;
const DEFAULT_ARGON2_PARALLELISM: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordHashConfig {
    pub memory_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
}

impl Default for PasswordHashConfig {
    fn default() -> Self {
        Self {
            memory_kib: DEFAULT_ARGON2_MEMORY_KIB,
            time_cost: DEFAULT_ARGON2_TIME_COST,
            parallelism: DEFAULT_ARGON2_PARALLELISM,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PasswordCalibration {
    pub config: PasswordHashConfig,
    pub elapsed_ms: u128,
}

#[derive(Clone)]
pub struct AuthService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    issuer: String,
    audience: String,
    access_ttl_seconds: i64,
    refresh_ttl_seconds: i64,
    rate_limiter: Arc<Mutex<AuthRateLimiter>>,
}

#[derive(Clone, Copy)]
pub enum AuthOperation {
    Login,
    Refresh,
    AdminLogin,
    AdminRefresh,
    AdminReauth,
    PayloadUpload,
    Purge,
    InvitationActivation,
}

struct AuthRateLimiter {
    windows: HashMap<String, RateWindow>,
    last_cleanup: Instant,
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            last_cleanup: Instant::now(),
        }
    }
}

struct RateWindow {
    started_at: Instant,
    requests: u32,
}

impl Default for RateWindow {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            requests: 0,
        }
    }
}

impl AuthRateLimiter {
    fn allow(&mut self, operation: AuthOperation, key: &str) -> bool {
        if self.last_cleanup.elapsed() >= AUTH_RATE_PERIOD {
            self.windows
                .retain(|_, window| window.started_at.elapsed() < AUTH_RATE_PERIOD);
            self.last_cleanup = Instant::now();
        }
        let rate_key = format!("{}:{key}", operation.as_str());
        if !self.windows.contains_key(&rate_key) && self.windows.len() >= AUTH_RATE_MAX_KEYS {
            return false;
        }
        let window = self.windows.entry(rate_key).or_default();
        if window.started_at.elapsed() >= AUTH_RATE_PERIOD {
            window.started_at = Instant::now();
            window.requests = 0;
        }
        if window.requests >= AUTH_RATE_LIMIT {
            return false;
        }
        window.requests += 1;
        true
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AccessClaims {
    pub sub: Uuid,
    pub role: String,
    #[serde(rename = "deviceId")]
    pub device_id: Uuid,
    pub iss: String,
    pub aud: String,
    pub jti: Uuid,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone, Debug)]
pub struct IssuedAccessToken {
    pub token: String,
    pub expires_in: i64,
}

#[derive(Clone, Debug)]
pub struct IssuedOpaqueToken {
    pub raw: String,
    pub hash: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid signing key")]
    InvalidKey,
    #[error("token operation failed")]
    Token,
    #[error("password operation failed")]
    Password,
    #[error("secure random generation failed")]
    Random,
}

impl AuthService {
    /// Creates an Ed25519 JWT service from a PKCS#8 PEM private key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key or token lifetime is invalid.
    pub fn from_private_key_pem(
        private_key_pem: &[u8],
        issuer: String,
        audience: String,
        access_ttl_seconds: i64,
        refresh_ttl_seconds: i64,
    ) -> Result<Self, AuthError> {
        if access_ttl_seconds <= 0 || refresh_ttl_seconds <= 0 {
            return Err(AuthError::InvalidKey);
        }
        configured_password_hash_config()?;
        let pem = pem::parse(private_key_pem).map_err(|_| AuthError::InvalidKey)?;
        let signing_key =
            SigningKey::from_pkcs8_der(pem.contents()).map_err(|_| AuthError::InvalidKey)?;
        Ok(Self {
            encoding_key: EncodingKey::from_ed_der(pem.contents()),
            decoding_key: DecodingKey::from_ed_der(signing_key.verifying_key().as_bytes()),
            issuer,
            audience,
            access_ttl_seconds,
            refresh_ttl_seconds,
            rate_limiter: Arc::new(Mutex::new(AuthRateLimiter::default())),
        })
    }

    /// # Errors
    ///
    /// Returns an error when signing fails.
    pub fn issue_access_token(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
    ) -> Result<IssuedAccessToken, AuthError> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = AccessClaims {
            sub: user_id,
            role: role.to_owned(),
            device_id,
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            jti: Uuid::new_v4(),
            iat: now,
            exp: now + self.access_ttl_seconds,
        };
        let token = encode(&Header::new(Algorithm::EdDSA), &claims, &self.encoding_key)
            .map_err(|_| AuthError::Token)?;
        Ok(IssuedAccessToken {
            token,
            expires_in: self.access_ttl_seconds,
        })
    }

    /// # Errors
    ///
    /// Returns an error when token validation fails.
    pub fn validate_access_token(&self, token: &str) -> Result<AccessClaims, AuthError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        decode::<AccessClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|_| AuthError::Token)
    }

    /// # Errors
    ///
    /// Returns an error when secure random generation fails.
    pub fn issue_refresh_token(&self) -> Result<IssuedOpaqueToken, AuthError> {
        random_opaque_token()
    }

    /// # Errors
    ///
    /// Returns an error when secure random generation fails.
    pub fn issue_invitation_token(&self) -> Result<IssuedOpaqueToken, AuthError> {
        random_opaque_token()
    }

    #[must_use]
    pub const fn refresh_ttl_seconds(&self) -> i64 {
        self.refresh_ttl_seconds
    }

    #[must_use]
    pub fn allow_auth_request(&self, operation: AuthOperation, key: &str) -> bool {
        self.rate_limiter
            .lock()
            .is_ok_and(|mut limiter| limiter.allow(operation, key))
    }

    /// Creates a short-lived token for a privileged operation.
    ///
    /// The caller must persist and consume the returned hash in a shared store.
    #[allow(clippy::missing_errors_doc)]
    pub fn issue_reauth_nonce(&self) -> Result<(IssuedOpaqueToken, i64), AuthError> {
        Ok((
            random_opaque_token()?,
            i64::try_from(REAUTH_NONCE_TTL.as_secs()).unwrap_or(i64::MAX),
        ))
    }
}

impl AuthOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Refresh => "refresh",
            Self::AdminLogin => "admin_login",
            Self::AdminRefresh => "admin_refresh",
            Self::AdminReauth => "admin_reauth",
            Self::PayloadUpload => "payload_upload",
            Self::Purge => "purge",
            Self::InvitationActivation => "invitation_activation",
        }
    }
}

/// # Errors
///
/// Returns an error when Argon2id cannot produce a password hash.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    hash_password_with_config(password, configured_password_hash_config()?)
}

fn hash_password_with_config(
    password: &str,
    config: PasswordHashConfig,
) -> Result<String, AuthError> {
    let mut salt = [0_u8; 16];
    getrandom::fill(&mut salt).map_err(|_| AuthError::Random)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| AuthError::Password)?;
    configured_argon2(config)?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Password)
}

#[must_use]
pub fn verify_password(password: &str, encoded_hash: &str) -> bool {
    PasswordHash::new(encoded_hash).is_ok_and(|hash| {
        configured_password_hash_config()
            .and_then(configured_argon2)
            .is_ok_and(|argon2| argon2.verify_password(password.as_bytes(), &hash).is_ok())
    })
}

fn configured_password_hash_config() -> Result<PasswordHashConfig, AuthError> {
    let defaults = PasswordHashConfig::default();
    let config = PasswordHashConfig {
        memory_kib: password_hash_env("TASKTIPS_ARGON2_MEMORY_KIB", defaults.memory_kib)?,
        time_cost: password_hash_env("TASKTIPS_ARGON2_TIME_COST", defaults.time_cost)?,
        parallelism: password_hash_env("TASKTIPS_ARGON2_PARALLELISM", defaults.parallelism)?,
    };
    validate_password_hash_config(config)
}

fn password_hash_env(name: &str, default: u32) -> Result<u32, AuthError> {
    match env::var(name) {
        Ok(value) => value.parse().map_err(|_| AuthError::Password),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(AuthError::Password),
    }
}

fn validate_password_hash_config(
    config: PasswordHashConfig,
) -> Result<PasswordHashConfig, AuthError> {
    if !(16_384..=1_048_576).contains(&config.memory_kib)
        || !(1..=10).contains(&config.time_cost)
        || !(1..=16).contains(&config.parallelism)
    {
        return Err(AuthError::Password);
    }
    Ok(config)
}

fn configured_argon2(config: PasswordHashConfig) -> Result<Argon2<'static>, AuthError> {
    let params = Params::new(
        config.memory_kib,
        config.time_cost,
        config.parallelism,
        None,
    )
    .map_err(|_| AuthError::Password)?;
    Ok(Argon2::new(
        Argon2Algorithm::Argon2id,
        Version::V0x13,
        params,
    ))
}

/// Benchmarks bounded Argon2id profiles using a fixed non-secret input.
///
/// The output is intended for deployment calibration and never contains a
/// user password or a generated hash.
#[allow(clippy::missing_errors_doc)]
pub fn calibrate_password_hash() -> Result<Vec<PasswordCalibration>, AuthError> {
    let profiles = [
        PasswordHashConfig {
            memory_kib: 16_384,
            time_cost: 1,
            parallelism: 1,
        },
        PasswordHashConfig {
            memory_kib: 32_768,
            time_cost: 2,
            parallelism: 1,
        },
        PasswordHashConfig {
            memory_kib: 65_536,
            time_cost: 3,
            parallelism: 1,
        },
    ];
    profiles
        .into_iter()
        .map(|config| {
            let started = std::time::Instant::now();
            let _ = hash_password_with_config("tasktips-argon2-calibration", config)?;
            Ok(PasswordCalibration {
                config,
                elapsed_ms: started.elapsed().as_millis(),
            })
        })
        .collect()
}

#[must_use]
pub fn opaque_token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn random_opaque_token() -> Result<IssuedOpaqueToken, AuthError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Random)?;
    let raw = URL_SAFE_NO_PAD.encode(bytes);
    Ok(IssuedOpaqueToken {
        hash: opaque_token_hash(&raw),
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AUTH_RATE_LIMIT, AUTH_RATE_MAX_KEYS, AuthOperation, AuthRateLimiter, AuthService,
        hash_password, opaque_token_hash, verify_password,
    };
    use der::pem::LineEnding;
    use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};

    #[test]
    fn passwords_are_argon2id_hashed_and_verified() {
        let hash =
            hash_password("correct horse battery staple").expect("password hashing should succeed");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn opaque_token_hash_is_stable_sha256() {
        assert_eq!(
            hex::encode(opaque_token_hash("token")),
            "3c469e9d6c5875d37a43f353d4f88e61fcf812c66eee3457465a40b0da4153e0"
        );
    }

    #[test]
    fn auth_rate_limits_are_partitioned_by_operation_and_key() {
        let mut limiter = AuthRateLimiter::default();
        for _ in 0..AUTH_RATE_LIMIT {
            assert!(limiter.allow(AuthOperation::Login, "client-a:user-a"));
        }
        assert!(!limiter.allow(AuthOperation::Login, "client-a:user-a"));
        assert!(limiter.allow(AuthOperation::Login, "client-a:user-b"));
        assert!(limiter.allow(AuthOperation::Refresh, "client-a:user-a"));
    }

    #[test]
    fn auth_rate_limiter_rejects_new_keys_at_capacity() {
        let mut limiter = AuthRateLimiter::default();
        for index in 0..AUTH_RATE_MAX_KEYS {
            assert!(limiter.allow(AuthOperation::Login, &format!("client-{index}")));
        }
        assert!(!limiter.allow(AuthOperation::Login, "overflow-client"));
        assert!(limiter.allow(AuthOperation::Login, "client-0"));
    }

    #[test]
    fn reauth_nonce_is_random_opaque_data() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("test key should encode");
        let auth = AuthService::from_private_key_pem(
            pem.as_bytes(),
            "issuer".to_owned(),
            "audience".to_owned(),
            900,
            3600,
        )
        .expect("auth service should build");
        let (nonce, expires_in) = auth.issue_reauth_nonce().expect("nonce should issue");

        assert_eq!(nonce.raw.len(), 43);
        assert_eq!(nonce.hash.len(), 32);
        assert_eq!(expires_in, 300);
    }
}
