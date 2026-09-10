//! Local credential store for CLI (and shared helpers).

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

/// On-disk credentials (mode 0600 when possible).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCredentials {
    /// Token handed to Hub as `Authorization: Bearer` (prefer id_token for OIDC).
    pub access_token: String,
    /// Optional raw id_token if distinct from access_token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Ticket provider: `oidc` | `wecom` (optional for backward compat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Resolve config directory: `ATLAS_BOT_CONFIG_DIR` or `dirs::config_dir()/atlas-bot`.
pub fn config_dir() -> Result<PathBuf, CredentialError> {
    if let Ok(override_dir) = std::env::var("ATLAS_BOT_CONFIG_DIR") {
        if !override_dir.is_empty() {
            return Ok(PathBuf::from(override_dir));
        }
    }
    let base = dirs::config_dir().ok_or_else(|| {
        CredentialError::Message("could not resolve config directory (set ATLAS_BOT_CONFIG_DIR)".into())
    })?;
    Ok(base.join("atlas-bot"))
}

pub fn credentials_path() -> Result<PathBuf, CredentialError> {
    Ok(config_dir()?.join("credentials.json"))
}

pub fn save_credentials(creds: &StoredCredentials) -> Result<PathBuf, CredentialError> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("credentials.json");
    let tmp = dir.join("credentials.json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(creds)?.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

pub fn load_credentials() -> Result<Option<StoredCredentials>, CredentialError> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

pub fn delete_credentials() -> Result<bool, CredentialError> {
    let path = credentials_path()?;
    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Load bearer string from credentials file (if any).
pub fn load_bearer() -> Result<Option<String>, CredentialError> {
    Ok(load_credentials()?.map(|c| c.access_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn roundtrip_with_override_dir() {
        let _g = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("atlas-bot-creds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("ATLAS_BOT_CONFIG_DIR", &dir);
        let creds = StoredCredentials {
            access_token: "tok".into(),
            id_token: Some("id".into()),
            refresh_token: None,
            expires_at: Some(123),
            token_type: Some("Bearer".into()),
            subject: Some("user_x".into()),
            provider: Some("wecom".into()),
        };
        let path = save_credentials(&creds).unwrap();
        assert_eq!(path, dir.join("credentials.json"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let loaded = load_credentials().unwrap().unwrap();
        assert_eq!(loaded, creds);
        assert_eq!(loaded.provider.as_deref(), Some("wecom"));
        assert!(delete_credentials().unwrap());
        assert!(load_credentials().unwrap().is_none());
        std::env::remove_var("ATLAS_BOT_CONFIG_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_without_provider_deserializes() {
        let raw = r#"{"access_token":"t","subject":"u"}"#;
        let c: StoredCredentials = serde_json::from_str(raw).unwrap();
        assert_eq!(c.access_token, "t");
        assert!(c.provider.is_none());
    }
}
