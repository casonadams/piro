//! Provider credentials in `~/.piro/auth.json`, keyed by provider name.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StoredCredential {
    #[serde(rename = "api_key")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuth {
        access: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh: Option<String>,
        /// Epoch milliseconds, already net of the refresh skew.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_email: Option<String>,
    },
}

impl StoredCredential {
    pub fn secret(&self) -> &str {
        match self {
            Self::ApiKey { key } => key,
            Self::OAuth { access, .. } => access,
        }
    }

    pub fn account_email(&self) -> Option<&str> {
        match self {
            Self::ApiKey { .. } => None,
            Self::OAuth { account_email, .. } => account_email.as_deref(),
        }
    }

    /// True when an OAuth token expires within `window_ms` of `now_ms`; API keys never expire.
    pub fn expires_within(&self, now_ms: i64, window_ms: i64) -> bool {
        matches!(self, Self::OAuth { expires: Some(at), .. } if now_ms + window_ms >= *at)
    }
}

#[derive(Debug, Default)]
pub struct AuthStore {
    path: PathBuf,
    entries: BTreeMap<String, StoredCredential>,
}

impl AuthStore {
    /// Reads the file as it is right now; a missing or empty file is an empty store.
    pub fn open(path: &Path) -> Result<Self, String> {
        let entries = match fs::read_to_string(path) {
            Ok(raw) if raw.trim().is_empty() => BTreeMap::new(),
            Ok(raw) => {
                serde_json::from_str(&raw).map_err(|e| format!("Failed to parse auth file {}: {e}", path.display()))?
            }
            Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("Failed to read auth file {}: {e}", path.display())),
        };
        Ok(Self {
            path: path.to_path_buf(),
            entries,
        })
    }

    pub fn get(&self, provider: &str) -> Option<&StoredCredential> {
        self.entries.get(provider)
    }

    pub fn set(&mut self, provider: &str, credential: StoredCredential) {
        self.entries.insert(provider.to_string(), credential);
    }

    /// Writes a private temp file and renames it over the store, so readers never see a partial file.
    pub fn save(&self) -> Result<(), String> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        create_private_dir(dir).map_err(|e| write_failure(&self.path, e))?;
        let json =
            serde_json::to_vec_pretty(&self.entries).map_err(|e| format!("Failed to serialize auth store: {e}"))?;
        let tmp = self.path.with_extension(format!("json.tmp{}", std::process::id()));
        let _ = fs::remove_file(&tmp);
        write_private(&tmp, &json)
            .and_then(|()| fs::rename(&tmp, &self.path))
            .map_err(|e| {
                let _ = fs::remove_file(&tmp);
                write_failure(&self.path, e)
            })
    }
}

/// `$HOME/.piro/auth.json`.
pub fn default_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set; cannot locate ~/.piro/auth.json")?;
    Ok(PathBuf::from(home).join(".piro").join("auth.json"))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn write_failure(path: &Path, error: std::io::Error) -> String {
    format!("Failed to write auth file {}: {error}", path.display())
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("piro-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("auth.json")
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let path = scratch("missing");
        assert!(AuthStore::open(&path).unwrap().get("claude").is_none());
    }

    #[test]
    fn round_trips_both_kinds_with_private_mode() {
        let path = scratch("roundtrip");
        let mut store = AuthStore::open(&path).unwrap();
        store.set(
            "claude",
            StoredCredential::OAuth {
                access: "a".into(),
                refresh: Some("r".into()),
                expires: Some(42),
                account_email: Some("me@example.com".into()),
            },
        );
        store.set("anthropic", StoredCredential::ApiKey { key: "k".into() });
        store.save().unwrap();

        let reopened = AuthStore::open(&path).unwrap();
        assert_eq!(reopened.get("claude"), store.get("claude"));
        assert_eq!(reopened.get("anthropic"), store.get("anthropic"));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"type\": \"oauth\"") && raw.contains("\"type\": \"api_key\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refreshes_only_inside_the_window() {
        let token = StoredCredential::OAuth {
            access: "a".into(),
            refresh: None,
            expires: Some(100_000),
            account_email: None,
        };
        assert!(!token.expires_within(0, 60_000));
        assert!(token.expires_within(40_000, 60_000));
        assert!(token.expires_within(200_000, 0));
        let no_expiry = StoredCredential::OAuth {
            access: "a".into(),
            refresh: None,
            expires: None,
            account_email: None,
        };
        assert!(!no_expiry.expires_within(i64::MAX / 2, 0));
        assert!(!StoredCredential::ApiKey { key: "k".into() }.expires_within(i64::MAX / 2, 0));
    }
}
