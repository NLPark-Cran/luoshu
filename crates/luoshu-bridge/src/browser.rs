//! Browser-control hook: implemented by the Tauri shell (drives the embedded
//! webview), mocked in tests. This is the Computer-Use entry point.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("browser control unavailable (no embedded webview)")]
    Unavailable,
    #[error("{0}")]
    Failed(String),
}

#[async_trait]
pub trait BrowserControl: Send + Sync {
    /// Evaluate JS in the embedded webview. Returns a JSON-encoded result
    /// when the platform supports return values, otherwise `"null"`.
    async fn eval(&self, script: &str) -> Result<String, BrowserError>;

    /// Navigate the embedded webview to `url`.
    async fn navigate(&self, url: &str) -> Result<(), BrowserError>;
}
