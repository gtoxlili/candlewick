//! Account credentials for market-data providers, in `credentials.json` next
//! to the settings, readable by the user only.

use std::{
    fmt, fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longbridge: Option<LongbridgeKeys>,
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
/// never readable by other users, not even halfway.
pub fn save(path: &Path, credentials: &Credentials) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    // A leftover would keep its own permissions.
    let _ = fs::remove_file(&tmp);
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(credentials)?)?;
    drop(file);
    fs::rename(&tmp, path)
}
