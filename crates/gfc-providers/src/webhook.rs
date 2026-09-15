//! Signed webhook verification. Signatures are checked against the raw body
//! before JSON is parsed.

use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebhookError {
    #[error("missing signature header")]
    MissingSignature,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("timestamp outside allowed skew")]
    TimestampSkew,
    #[error("unsupported webhook")]
    Unsupported,
}

pub fn verify_github_hmac(
    secret: &str,
    body: &[u8],
    signature_header: Option<&str>,
) -> Result<(), WebhookError> {
    let header = signature_header.ok_or(WebhookError::MissingSignature)?;
    let hex = header
        .strip_prefix("sha256=")
        .ok_or(WebhookError::InvalidSignature)?;
    let expected = hex::decode(hex).map_err(|_| WebhookError::InvalidSignature)?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| WebhookError::InvalidSignature)?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| WebhookError::InvalidSignature)
}

pub fn verify_gitlab_token(expected: &str, header: Option<&str>) -> Result<(), WebhookError> {
    let got = header.ok_or(WebhookError::MissingSignature)?;
    if got.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(WebhookError::InvalidSignature)
    }
}

/// Origin webhook: Ed25519 over SHA-256("{id}.{timestamp}." || body).
/// `public_key` is the 32-byte Ed25519 verifying key.
pub fn verify_origin_ed25519(
    body: &[u8],
    webhook_id: Option<&str>,
    webhook_timestamp: Option<&str>,
    webhook_signature: Option<&str>,
    public_key: &[u8; 32],
    now_unix: i64,
) -> Result<(), WebhookError> {
    let id = webhook_id.ok_or(WebhookError::MissingSignature)?;
    let ts_raw = webhook_timestamp.ok_or(WebhookError::MissingSignature)?;
    let ts: i64 = ts_raw.parse().map_err(|_| WebhookError::InvalidSignature)?;
    if (now_unix - ts).abs() > 300 {
        return Err(WebhookError::TimestampSkew);
    }
    let header = webhook_signature.ok_or(WebhookError::MissingSignature)?;
    let b64 = header
        .split_whitespace()
        .find_map(|part| part.strip_prefix("v1ed,"))
        .ok_or(WebhookError::InvalidSignature)?;
    let sig_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .map_err(|_| WebhookError::InvalidSignature)?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| WebhookError::InvalidSignature)?;
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update(b".");
    hasher.update(ts_raw.as_bytes());
    hasher.update(b".");
    hasher.update(body);
    let digest_hex = hex::encode(hasher.finalize());
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| WebhookError::InvalidSignature)?;
    key.verify_strict(digest_hex.as_bytes(), &sig)
        .map_err(|_| WebhookError::InvalidSignature)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use hmac::Mac;

    #[test]
    fn github_accepts_valid_hmac() {
        let secret = "topsecret";
        let body = b"{\"zen\":\"ok\"}";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let hex = hex::encode(mac.finalize().into_bytes());
        let header = format!("sha256={hex}");
        assert!(verify_github_hmac(secret, body, Some(&header)).is_ok());
        assert_eq!(
            verify_github_hmac(secret, body, Some("sha256=00")).unwrap_err(),
            WebhookError::InvalidSignature
        );
    }

    #[test]
    fn gitlab_token_match() {
        assert!(verify_gitlab_token("abc", Some("abc")).is_ok());
        assert!(verify_gitlab_token("abc", Some("nope")).is_err());
        assert!(verify_gitlab_token("abc", None).is_err());
    }

    #[test]
    fn origin_ed25519_round_trip() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let vk = signing.verifying_key();
        let body = b"{\"type\":\"repository.check_run.completed\"}";
        let id = "deliv-1";
        let ts = "1700000000";
        let mut hasher = Sha256::new();
        hasher.update(id.as_bytes());
        hasher.update(b".");
        hasher.update(ts.as_bytes());
        hasher.update(b".");
        hasher.update(body);
        let digest_hex = hex::encode(hasher.finalize());
        let sig = signing.sign(digest_hex.as_bytes());
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig.to_bytes());
        let header = format!("v1ed,{b64}");
        assert!(
            verify_origin_ed25519(
                body,
                Some(id),
                Some(ts),
                Some(&header),
                vk.as_bytes(),
                1_700_000_000,
            )
            .is_ok()
        );
        assert_eq!(
            verify_origin_ed25519(
                body,
                Some(id),
                Some(ts),
                Some(&header),
                vk.as_bytes(),
                1_700_000_000 + 400,
            )
            .unwrap_err(),
            WebhookError::TimestampSkew
        );
    }
}
