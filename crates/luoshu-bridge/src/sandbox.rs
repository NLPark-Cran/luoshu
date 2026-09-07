//! Filesystem sandbox: allowed roots + hidden-component and sensitive-pattern
//! refusal. All `luoshu_fs_*` tools must pass through [`Sandbox::check_read`]
//! / [`Sandbox::check_write`].

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("path escapes the allowed roots")]
    OutsideRoots,
    #[error("hidden path components (.{name}) are not accessible")]
    HiddenComponent { name: String },
    #[error("path matches a sensitive pattern ({pattern})")]
    Sensitive { pattern: String },
    #[error("path does not exist and cannot be canonicalized")]
    NotFound,
}

/// File/dir names that are always refused, anywhere under a root.
const SENSITIVE_EXACT: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".gpg",
    ".kube",
    ".docker",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "credentials",
    // Never leak the bridge's own state (contains the auth token).
    "bridge.json",
    // Cloud enrollment (contains the device token; ADR 004).
    "device.json",
];

/// Prefixes refused (case-insensitive): `.env`, `.env.local`, ...
const SENSITIVE_PREFIXES: &[&str] = &[".env"];

/// Extensions refused: private keys & cert bundles.
const SENSITIVE_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "kdbx"];

pub struct Sandbox {
    roots: Vec<PathBuf>,
}

impl Sandbox {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let roots = roots
            .into_iter()
            .map(|r| r.canonicalize().unwrap_or(r))
            .collect();
        Self { roots }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Validate an existing path for reading. Returns the canonicalized path.
    pub fn check_read(&self, path: &Path) -> Result<PathBuf, SandboxError> {
        let canonical = path.canonicalize().map_err(|_| SandboxError::NotFound)?;
        self.check_canonical(&canonical)?;
        Ok(canonical)
    }

    /// Validate a (possibly non-existent) path for writing: canonicalize the
    /// deepest existing ancestor, re-attach the remainder, then validate.
    pub fn check_write(&self, path: &Path) -> Result<PathBuf, SandboxError> {
        let mut ancestor = path.to_path_buf();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        loop {
            match ancestor.canonicalize() {
                Ok(c) => {
                    let mut full = c;
                    for part in tail.iter().rev() {
                        full.push(part);
                    }
                    self.check_canonical(&full)?;
                    return Ok(full);
                }
                Err(_) => {
                    let Some(file_name) = ancestor.file_name() else {
                        return Err(SandboxError::NotFound);
                    };
                    tail.push(file_name.to_os_string());
                    ancestor.pop();
                }
            }
        }
    }

    fn check_canonical(&self, path: &Path) -> Result<(), SandboxError> {
        let root = self
            .roots
            .iter()
            .find(|r| path.starts_with(r))
            .ok_or(SandboxError::OutsideRoots)?;

        // Sensitive patterns apply everywhere (even inside the root itself).
        for comp in path.components() {
            if let Component::Normal(name) = comp {
                check_sensitive(&name.to_string_lossy())?;
            }
        }

        // Hidden components are refused *below* the root (a user may
        // explicitly whitelist a dot-dir root via allowed_roots.json).
        let rel = path.strip_prefix(root).unwrap_or(path);
        for comp in rel.components() {
            if let Component::Normal(name) = comp {
                let name = name.to_string_lossy();
                if name.starts_with('.') {
                    return Err(SandboxError::HiddenComponent {
                        name: name.trim_start_matches('.').to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn check_sensitive(name: &str) -> Result<(), SandboxError> {
    let lower = name.to_ascii_lowercase();
    if SENSITIVE_EXACT.contains(&lower.as_str()) {
        return Err(SandboxError::Sensitive {
            pattern: lower.clone(),
        });
    }
    if SENSITIVE_PREFIXES
        .iter()
        .any(|p| lower == *p || lower.starts_with(&format!("{p}.")))
    {
        return Err(SandboxError::Sensitive { pattern: lower });
    }
    if let Some(ext) = lower.rsplit('.').next() {
        if lower.contains('.') && SENSITIVE_EXTENSIONS.contains(&ext) {
            return Err(SandboxError::Sensitive {
                pattern: format!("*.{ext}"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Sandbox) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("docs")).unwrap();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::write(home.join("notes.txt"), "hi").unwrap();
        fs::write(home.join("docs/a.md"), "a").unwrap();
        fs::write(home.join(".ssh/id_rsa"), "KEY").unwrap();
        fs::write(home.join(".env"), "SECRET=1").unwrap();
        fs::write(home.join("cert.pem"), "PEM").unwrap();
        fs::write(home.join("docs/.env.production"), "SECRET=1").unwrap();
        let sandbox = Sandbox::new(vec![home]);
        (tmp, sandbox)
    }

    #[test]
    fn allows_normal_files() {
        let (_t, s) = fixture();
        assert!(s.check_read(Path::new("notes.txt")).is_err()); // relative → NotFound
        let home = &s.roots()[0];
        assert!(s.check_read(&home.join("notes.txt")).is_ok());
        assert!(s.check_read(&home.join("docs/a.md")).is_ok());
    }

    #[test]
    fn refuses_ssh_dir() {
        let (_t, s) = fixture();
        let home = &s.roots()[0];
        let err = s.check_read(&home.join(".ssh/id_rsa")).unwrap_err();
        assert!(matches!(
            err,
            SandboxError::Sensitive { .. } | SandboxError::HiddenComponent { .. }
        ));
    }

    #[test]
    fn refuses_dotenv_and_keys() {
        let (_t, s) = fixture();
        let home = &s.roots()[0];
        assert!(matches!(
            s.check_read(&home.join(".env")),
            Err(SandboxError::Sensitive { .. })
        ));
        assert!(matches!(
            s.check_read(&home.join("cert.pem")),
            Err(SandboxError::Sensitive { .. })
        ));
        let r = s.check_read(&home.join("docs/.env.production"));
        eprintln!("DEBUG result = {r:?}");
        assert!(matches!(r, Err(SandboxError::Sensitive { .. })));
    }

    #[test]
    fn refuses_escape_outside_roots() {
        let (_t, s) = fixture();
        let home = &s.roots()[0];
        // Symlink escape attempt: link inside home pointing at /etc/passwd.
        let link = home.join("escape.txt");
        std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();
        assert!(matches!(
            s.check_read(&link),
            Err(SandboxError::OutsideRoots)
        ));
        // `..` traversal collapses after canonicalization → still outside.
        assert!(matches!(
            s.check_read(&home.join("../outside.txt")),
            Err(SandboxError::NotFound | SandboxError::OutsideRoots)
        ));
    }

    #[test]
    fn write_to_new_file_in_allowed_dir() {
        let (_t, s) = fixture();
        let home = &s.roots()[0];
        let target = home.join("docs/new/nested.txt");
        assert!(s.check_write(&target).is_ok());
        // Writing a new hidden file is refused even though it doesn't exist.
        assert!(s.check_write(&home.join("docs/.hidden")).is_err());
        // New key file refused.
        assert!(s.check_write(&home.join("docs/server.key")).is_err());
    }
}
