//! Configuration & per-install state (`~/.luoshu/`).
//!
//! - `bridge.json`        — {port, token, pid, started_at}, mode 0600, rewritten each run.
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
