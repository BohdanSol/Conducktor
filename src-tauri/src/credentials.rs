//! Reads the OAuth access token that Claude Code itself stores on the
//! machine. We never write, refresh, or otherwise mutate these
//! credentials — Claude Code owns that lifecycle and keeps the token
//! fresh on its own. We just re-read it whenever ours looks stale.

use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// Re-read the token this long before its reported expiry, so we don't
/// race a request against the exact expiry instant.
const EXPIRY_SAFETY_MARGIN_SECS: i64 = 60;

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_token: String,
    /// Milliseconds since epoch, as stored by Claude Code.
    pub expires_at_ms: i64,
}

impl Credentials {
    pub fn is_expired(&self) -> bool {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(i64::MAX);
        now_ms >= self.expires_at_ms - EXPIRY_SAFETY_MARGIN_SECS * 1000
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeAiOauth {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt")]
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: ClaudeAiOauth,
}

#[derive(Debug, Clone)]
pub enum CredError {
    /// Neither the Keychain item nor the fallback file could be found.
    NotLoggedIn,
    /// The Keychain entry exists but the OS denied us access to it.
    KeychainDenied,
    /// Something was found but didn't parse as expected.
    ParseError(String),
}

impl std::fmt::Display for CredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredError::NotLoggedIn => write!(f, "not logged in to Claude Code"),
            CredError::KeychainDenied => write!(f, "Keychain access denied"),
            CredError::ParseError(msg) => write!(f, "failed to parse credentials: {msg}"),
        }
    }
}

/// Reads the credential JSON from macOS Keychain, falling back to
/// `~/.claude/.credentials.json` if the Keychain item isn't present
/// (e.g. on non-macOS, or an unusual install).
fn read_raw_json() -> Result<String, CredError> {
    let mut keychain_denied = false;

    if cfg!(target_os = "macos") {
        let output = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !text.is_empty() {
                    return Ok(text);
                }
            }
            Ok(out) => {
                // Exit code 44 = item not found (not an error worth
                // surfacing — just means the fallback file is next).
                // Anything else usually means the item exists but access
                // was denied (locked keychain, user declined the prompt).
                let code = out.status.code().unwrap_or(-1);
                if code != 44 {
                    keychain_denied = true;
                }
            }
            Err(_) => {
                // `security` not on PATH at all — fall through to file.
            }
        }
    }

    let fallback = fallback_path();
    match std::fs::read_to_string(&fallback) {
        Ok(text) => Ok(text),
        Err(_) if keychain_denied => Err(CredError::KeychainDenied),
        Err(_) => Err(CredError::NotLoggedIn),
    }
}

fn fallback_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".claude").join(".credentials.json")
}

fn parse_credentials(raw: &str) -> Result<Credentials, CredError> {
    let parsed: CredentialsFile =
        serde_json::from_str(raw).map_err(|e| CredError::ParseError(e.to_string()))?;
    Ok(Credentials {
        access_token: parsed.claude_ai_oauth.access_token,
        expires_at_ms: parsed.claude_ai_oauth.expires_at,
    })
}

/// Blocking read of the credentials from disk/Keychain. Runs the
/// `security` subprocess, so callers should invoke this via
/// `tokio::task::spawn_blocking` rather than calling it directly from
/// an async context.
pub fn read_credentials_blocking() -> Result<Credentials, CredError> {
    let raw = read_raw_json()?;
    parse_credentials(&raw)
}
