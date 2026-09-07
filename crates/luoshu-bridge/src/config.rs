//! Configuration & per-install state (`~/.luoshu/`).
//!
//! - `bridge.json`        — {port, token, pid, started_at}, mode 0600, rewritten each run.
//! - `device.json`        — cloud enrollment {cloud_url, device_id, device_token, name},
//!                          mode 0600, written once at device registration (ADR 004).
//! - `allowed_roots.json` — {"roots": ["/abs/path", ...]}, optional; overrides default roots.
//! - `screenshots/`       — PNG output of `luoshu_screenshot`.

use std::fs;
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};

/// Env var overriding the luoshu home dir (used by tests and power users).
pub const LUOSHU_HOME_ENV: &str = "LUOSHU_HOME";

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// `~/.luoshu` (or `$LUOSHU_HOME`).
    pub home_dir: PathBuf,
    /// Random bearer token for this run.
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeStateFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: u64,
}

#[derive(Debug, Deserialize)]
struct AllowedRootsFile {
    roots: Vec<String>,
}

/// Cloud enrollment for the reverse tunnel (ADR 004). The `device_token` is
/// issued once by `POST /api/v2/devices` on the cran-code server and stored
/// here (0600); the server only keeps its SHA-256 hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCredentials {
    /// e.g. `https://crys.tt2.li` (scheme rewritten to ws/wss for dialing).
    pub cloud_url: String,
    pub device_id: String,
    pub device_token: String,
    #[serde(default)]
    pub name: Option<String>,
}

impl DeviceCredentials {
    /// WebSocket URL of the cloud tunnel endpoint.
    pub fn tunnel_url(&self) -> String {
        let base = self.cloud_url.trim_end_matches('/');
        let ws_base = base
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!(
            "{ws_base}/api/v2/devices/{}/tunnel?token={}",
            self.device_id, self.device_token
        )
    }
}

impl BridgeConfig {
    /// Production config: token freshly generated per run.
    pub fn from_env() -> std::io::Result<Self> {
        let home_dir = match std::env::var_os(LUOSHU_HOME_ENV) {
            Some(p) => PathBuf::from(p),
            None => dirs::home_dir()
                .ok_or_else(|| std::io::Error::other("cannot locate home directory"))?
                .join(".luoshu"),
        };
        fs::create_dir_all(&home_dir)?;
        Ok(Self {
            home_dir,
            token: generate_token(),
        })
    }

    /// Test config rooted at an arbitrary dir with a fixed token.
    pub fn for_test(home_dir: PathBuf, token: &str) -> Self {
        Self {
            home_dir,
            token: token.to_string(),
        }
    }

    pub fn state_path(&self) -> PathBuf {
        self.home_dir.join("bridge.json")
    }

    pub fn allowed_roots_path(&self) -> PathBuf {
        self.home_dir.join("allowed_roots.json")
    }

    pub fn screenshots_dir(&self) -> PathBuf {
        self.home_dir.join("screenshots")
    }

    pub fn device_path(&self) -> PathBuf {
        self.home_dir.join("device.json")
    }

    /// Load cloud enrollment, if the device has been registered.
    pub fn load_device_credentials(&self) -> Option<DeviceCredentials> {
        let content = fs::read_to_string(self.device_path()).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Persist cloud enrollment with 0600 permissions.
    pub fn write_device_credentials(&self, creds: &DeviceCredentials) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(creds).map_err(std::io::Error::other)?;
        let path = self.device_path();
        fs::write(&path, json)?;
        set_mode_0600(&path)?;
        Ok(())
    }

    /// Persist `{port, token, pid, started_at}` with 0600 permissions.
    pub fn write_state_file(&self, port: u16) -> std::io::Result<()> {
        let state = BridgeStateFile {
            port,
            token: self.token.clone(),
            pid: std::process::id(),
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(std::io::Error::other)?;
        let path = self.state_path();
        fs::write(&path, json)?;
        set_mode_0600(&path)?;
        Ok(())
    }

    /// Allowed filesystem roots.
    ///
    /// Default: the user's home directory (hidden path components and
    /// sensitive patterns are still refused by the sandbox). If
    /// `allowed_roots.json` exists and parses with a non-empty `roots` list,
    /// it *replaces* the default.
    pub fn allowed_roots(&self) -> Vec<PathBuf> {
        if let Ok(content) = fs::read_to_string(self.allowed_roots_path()) {
            if let Ok(file) = serde_json::from_str::<AllowedRootsFile>(&content) {
                if !file.roots.is_empty() {
                    return file.roots.iter().map(PathBuf::from).collect();
                }
            }
        }
        let home = match std::env::var_os(LUOSHU_HOME_ENV) {
            // In test mode LUOSHU_HOME points at ~/.luoshu itself; treat the
            // real HOME (its parent) as the sandboxed root.
            Some(_) => self
                .home_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.home_dir.clone()),
            None => dirs::home_dir().unwrap_or_else(|| self.home_dir.clone()),
        };
        vec![home]
    }
}

/// Cryptographically random 64-char hex token.
pub fn generate_token() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn set_mode_0600(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_mode_0600(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_random_hex() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn state_file_written_with_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BridgeConfig::for_test(tmp.path().join(".luoshu"), "tok");
        fs::create_dir_all(&cfg.home_dir).unwrap();
        cfg.write_state_file(4321).unwrap();
        let raw = fs::read_to_string(cfg.state_path()).unwrap();
        let parsed: BridgeStateFile = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.port, 4321);
        assert_eq!(parsed.token, "tok");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(cfg.state_path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn device_credentials_roundtrip_with_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BridgeConfig::for_test(tmp.path().join(".luoshu"), "tok");
        fs::create_dir_all(&cfg.home_dir).unwrap();
        assert!(cfg.load_device_credentials().is_none());
        let creds = DeviceCredentials {
            cloud_url: "https://crys.tt2.li".into(),
            device_id: "d1".into(),
            device_token: "secret".into(),
            name: Some("rig".into()),
        };
        cfg.write_device_credentials(&creds).unwrap();
        let loaded = cfg.load_device_credentials().unwrap();
        assert_eq!(loaded.device_id, "d1");
        assert_eq!(loaded.device_token, "secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(cfg.device_path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn allowed_roots_file_overrides_default() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BridgeConfig::for_test(tmp.path().to_path_buf(), "t");
        fs::write(
            cfg.allowed_roots_path(),
            r#"{"roots": ["/tmp/a", "/tmp/b"]}"#,
        )
        .unwrap();
        assert_eq!(
            cfg.allowed_roots(),
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
    }
}
