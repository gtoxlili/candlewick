//! Longbridge's REST side: the one-time password that opens the quote socket,
//! and access-token refresh. Every request is signed with the app secret.

use std::{
    fmt::Write as _,
    sync::LazyLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ring::{digest, hmac};
use serde::{Deserialize, de::DeserializeOwned};

use super::clock;
use crate::{credentials::LongbridgeKeys, http, market::Error, net};

pub struct Hosts {
    pub http: &'static str,
    pub quote: &'static str,
}

const GLOBAL: Hosts =
    Hosts { http: "openapi.longbridge.com", quote: "openapi-quote.longbridge.com" };
const MAINLAND: Hosts =
    Hosts { http: "openapi.longbridge.cn", quote: "openapi-quote.longbridge.cn" };

/// Mainland China reaches Longbridge through its `.cn` hosts, which only
/// answer there; the official SDK tells the two apart the same way.
pub async fn hosts() -> &'static Hosts {
    static MAINLAND_PROBE: LazyLock<tokio::sync::OnceCell<bool>> =
        LazyLock::new(tokio::sync::OnceCell::new);
    let mainland = MAINLAND_PROBE
        .get_or_init(|| async {
            let probe = http::send(|client| {
                client.get("https://geotest.lbkrs.com").timeout(Duration::from_secs(5))
            });
            probe.await.is_ok_and(|response| response.status().is_success())
        })
        .await;
    if *mainland { &MAINLAND } else { &GLOBAL }
}

/// Opens one quote socket; valid for a single authentication.
pub async fn one_time_password(keys: &LongbridgeKeys) -> Result<String, Error> {
    #[derive(Deserialize)]
    struct Otp {
        otp: String,
        limit: i64,
        online: i64,
    }
    let otp: Otp = get(keys, "/v1/socket/token", "").await?;
    if otp.online >= otp.limit {
        return Err(Error::Message(format!(
            "长桥连接数已满（{} / {}），请关闭其他使用这组凭证的程序",
            otp.online, otp.limit
        )));
    }
    Ok(otp.otp)
}

/// A new access token valid until `expires` (epoch seconds).
pub async fn refresh_token(keys: &LongbridgeKeys, expires: i64) -> Result<String, Error> {
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let query = format!("expired_at={}", net::percent_encode(&clock::rfc3339(expires)));
    let token: Token = get(keys, "/v1/token/refresh", &query).await?;
    Ok(token.token)
}

/// When the access token expires (epoch seconds), read from its claims.
/// Tokens look like `hk_m_<JWT>`.
pub fn token_expiry(token: &str) -> Option<i64> {
    #[derive(Deserialize)]
    struct Claims {
        exp: i64,
    }
    let jwt = &token[token.find("eyJ")?..];
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice::<Claims>(&base64url(payload)?).ok().map(|claims| claims.exp)
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

async fn get<T: DeserializeOwned>(
    keys: &LongbridgeKeys,
    path: &str,
    query: &str,
) -> Result<T, Error> {
    let hosts = hosts().await;
    let timestamp = now().to_string();
    let signature = sign("GET", path, query, keys, &timestamp);
    let mut url = format!("https://{}{path}", hosts.http);
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    let response = http::send(|client| {
        client
            .get(&url)
            .header("X-Api-Key", &keys.app_key)
            .header("Authorization", &keys.access_token)
            .header("X-Timestamp", &timestamp)
            .header("X-Api-Signature", &signature)
            .header("x-dc-region", region(keys))
    })
    .await?;
    let status = response.status();
    // Failures come back as the same envelope, with an error code.
    let envelope: Envelope<T> =
        response.json().await.map_err(|_| http::Error::Status(status.as_u16()))?;
    match envelope {
        Envelope { code: 0, data: Some(data), .. } => Ok(data),
        Envelope { code, message, .. } => {
            log::warn!("longbridge {path}: {code} {message}");
            let message = if message.is_empty() { format!("错误 {code}") } else { message };
            Err(Error::Message(format!("长桥：{message}")))
        }
    }
}

/// The data center serving the credentials: `us` for keys issued there.
fn region(keys: &LongbridgeKeys) -> &'static str {
    let us =
        [&keys.app_key, &keys.app_secret, &keys.access_token].iter().any(|k| k.starts_with("us_"));
    if us { "us" } else { "ap" }
}

/// Longbridge's HMAC-SHA256 request signature over the method, path, query
/// and the key, token and timestamp headers.
fn sign(method: &str, path: &str, query: &str, keys: &LongbridgeKeys, timestamp: &str) -> String {
    const SIGNED_HEADERS: &str = "authorization;x-api-key;x-timestamp";
    let values = format!(
        "authorization:{}\nx-api-key:{}\nx-timestamp:{timestamp}\n",
        keys.access_token, keys.app_key
    );
    let canonical = format!("{method}|{path}|{query}|{values}|{SIGNED_HEADERS}|");
    let digest = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, canonical.as_bytes());
    let to_sign = format!("HMAC-SHA256|{}", hex(digest.as_ref()));
    let key = hmac::Key::new(hmac::HMAC_SHA256, keys.app_secret.as_bytes());
    let signature = hmac::sign(&key, to_sign.as_bytes());
    format!("HMAC-SHA256 SignedHeaders={SIGNED_HEADERS}, Signature={}", hex(signature.as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Unpadded base64url, as in JWTs.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes().filter(|&b| b != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        buffer = buffer << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_expiry_of_a_token() {
        // A token's shape with a made-up payload: {"exp":1800000000}
        let token = "hk_m_eyJhbGciOiJSUzI1NiJ9.eyJleHAiOjE4MDAwMDAwMDB9.c2lnbmF0dXJl";
        assert_eq!(token_expiry(token), Some(1_800_000_000));
        assert_eq!(token_expiry("not a token"), None);
    }

    #[test]
    fn signs_like_the_sdk() {
        let keys = LongbridgeKeys {
            app_key: "key".into(),
            app_secret: "secret".into(),
            access_token: "token".into(),
        };
        let header = sign("GET", "/v1/socket/token", "", &keys, "1700000000");
        assert!(header.starts_with(
            "HMAC-SHA256 SignedHeaders=authorization;x-api-key;x-timestamp, Signature="
        ));
        assert_eq!(header.rsplit('=').next().map(str::len), Some(64));
    }
}
