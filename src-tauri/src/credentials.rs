//! Account credentials, in `credentials.json` next to the settings, readable
//! by the user only: Longbridge's for stock quotes, and exchange API keys for
//! holdings, which the app only ever reads with.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::market::ProviderId;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longbridge: Option<LongbridgeKeys>,
    /// By crypto exchange.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exchanges: BTreeMap<ProviderId, ApiKey>,
}

/// An API key from an exchange's API management page.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    /// OKX's, chosen when the key was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

// Keeps the secrets out of logs.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey { .. }")
    }
}

/// From the Longbridge developer center: the app's key and secret, and an
/// access token that expires (after 90 days unless refreshed).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongbridgeKeys {
    pub app_key: String,
    pub app_secret: String,
    pub access_token: String,
}

// Keeps the secrets out of logs.
impl fmt::Debug for LongbridgeKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LongbridgeKeys { .. }")
    }
}

/// A missing or unreadable file means no credentials.
pub fn load(path: &Path) -> Credentials {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::warn!("credentials file unreadable: {e}");
            Credentials::default()
        }),
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("cannot read credentials: {e}");
            }
            Credentials::default()
        }
    }
}

/// Writes through a temp file created with mode 0600, so the secrets are
/// never readable by other users, not even halfway. On Windows the user's
/// profile folder is theirs alone already.
pub fn save(path: &Path, credentials: &Credentials) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    // A leftover would keep its own permissions.
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(credentials)?)?;
    drop(file);
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads() {
        let dir =
            std::env::temp_dir().join(format!("candlewick-credentials-{}", std::process::id()));
        let path = dir.join("credentials.json");
        let credentials = Credentials {
            longbridge: Some(LongbridgeKeys {
                app_key: "key".into(),
                app_secret: "secret".into(),
                access_token: "token".into(),
            }),
            exchanges: [(
                ProviderId::Okx,
                ApiKey { key: "key".into(), secret: "secret".into(), passphrase: Some("p".into()) },
            )]
            .into(),
        };
        save(&path, &credentials).unwrap();
        // Replacing an existing file goes through the same temp file.
        save(&path, &credentials).unwrap();
        assert_eq!(load(&path), credentials);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(load(&path), Credentials::default());
    }
}
