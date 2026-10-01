//! What signing requests to exchange and broker APIs takes: HMAC-SHA256 (via
//! ring) and its text forms.

use base64::Engine as _;
use ring::hmac;

pub fn hmac_sha256(secret: &str, message: &str) -> hmac::Tag {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()), message.as_bytes())
}

/// Lowercase.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Standard alphabet, padded.
pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
