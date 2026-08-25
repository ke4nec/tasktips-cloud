use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct CursorSigner {
    secret: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorKind {
    Bootstrap,
    Pull,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorClaims {
    pub schema_version: u8,
    pub kind: CursorKind,
    pub project_id: Uuid,
    pub owner_user_id: Uuid,
    pub generation: i64,
    pub change_sequence: i64,
    pub offset: i64,
    pub issued_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("cursor signing secret is too short")]
    Secret,
    #[error("cursor is invalid")]
    Invalid,
}

impl CursorSigner {
    /// # Errors
    ///
    /// Returns an error when the signing secret has less than 32 bytes.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, CursorError> {
        let secret = secret.into();
        if secret.len() < 32 {
            return Err(CursorError::Secret);
        }
        Ok(Self { secret })
    }

    /// # Errors
    ///
    /// Returns an error only if cursor serialization or HMAC setup fails.
    pub fn sign(&self, mut claims: CursorClaims) -> Result<String, CursorError> {
        claims.schema_version = 1;
        claims.issued_at = OffsetDateTime::now_utc().unix_timestamp();
        let payload = serde_json::to_vec(&claims).map_err(|_| CursorError::Invalid)?;
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let signature =
            URL_SAFE_NO_PAD.encode(hmac_sha256(&self.secret, encoded_payload.as_bytes()));
        Ok(format!("{encoded_payload}.{signature}"))
    }

    /// # Errors
    ///
    /// Returns an error for malformed, tampered, or unsupported cursors.
    pub fn verify(&self, cursor: &str) -> Result<CursorClaims, CursorError> {
        let (payload, signature) = cursor.split_once('.').ok_or(CursorError::Invalid)?;
        if signature.contains('.') {
            return Err(CursorError::Invalid);
        }
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| CursorError::Invalid)?;
        let expected = hmac_sha256(&self.secret, payload.as_bytes());
        if signature.len() != expected.len()
            || signature
                .iter()
                .zip(expected)
                .fold(0_u8, |difference, (left, right)| {
                    difference | (left ^ right)
                })
                != 0
        {
            return Err(CursorError::Invalid);
        }
        let claims: CursorClaims = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| CursorError::Invalid)
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|_| CursorError::Invalid))?;
        if claims.schema_version != 1 {
            return Err(CursorError::Invalid);
        }
        Ok(claims)
    }
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for ((inner, outer), key_byte) in inner_pad
        .iter_mut()
        .zip(outer_pad.iter_mut())
        .zip(normalized_key)
    {
        *inner ^= key_byte;
        *outer ^= key_byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{CursorClaims, CursorKind, CursorSigner};
    use uuid::Uuid;

    #[test]
    fn cursor_rejects_tampering() {
        let signer = CursorSigner::new([7_u8; 32]).expect("secret should be accepted");
        let claims = CursorClaims {
            schema_version: 0,
            kind: CursorKind::Pull,
            project_id: Uuid::nil(),
            owner_user_id: Uuid::from_u128(1),
            generation: 3,
            change_sequence: 42,
            offset: 0,
            issued_at: 0,
        };
        let cursor = signer.sign(claims.clone()).expect("cursor should sign");
        let verified = signer.verify(&cursor).expect("cursor should verify");
        assert_eq!(verified.project_id, claims.project_id);
        let mut tampered = cursor.into_bytes();
        tampered[4] = if tampered[4] == b'a' { b'b' } else { b'a' };
        assert!(
            signer
                .verify(&String::from_utf8(tampered).unwrap())
                .is_err()
        );
    }
}
