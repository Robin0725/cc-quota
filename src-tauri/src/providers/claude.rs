use std::{fs, path::PathBuf, process::Stdio, time::Duration};

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::{process::Command, time::timeout};

use crate::models::{ProviderSnapshot, ScopedWindow, UsageWindow};
use crate::providers::{CapsulePalette, ProviderAdapter, ProviderDescriptor};

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    id: "claude",
    display_name: "Claude",
    abbreviation: "CL",
    palette: CapsulePalette {
        border: [91, 49, 37, 255],
        track: [91, 49, 37, 255],
        fill_top: [184, 90, 58, 255],
        fill_bottom: [184, 90, 58, 255],
    },
    accent_hex: "#b85a3a",
    focus_hints: &["claude"],
};

pub struct ClaudeAdapter;

#[async_trait::async_trait]
impl ProviderAdapter for ClaudeAdapter {
    fn descriptor(&self) -> &'static ProviderDescriptor {
        &DESCRIPTOR
    }

    fn is_configured(&self) -> bool {
        has_local_login()
    }

    fn activity_paths(&self) -> Vec<PathBuf> {
        projects_path().into_iter().collect()
    }

    async fn fetch_snapshot(&self, client: &reqwest::Client) -> ProviderSnapshot {
        fetch_snapshot(client).await
    }
}

/// Mirrors the sources `load_auth` reads from, without ever retrieving the secret: the keychain is
/// queried for the entry's existence only (no `-w`), so nothing decrypted enters this process.
fn has_local_login() -> bool {
    if std::env::var("CLAUDE_CODE_OAUTH_TOKEN")
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    if read_file_credentials()
        .map(|raw| parse_auth(&raw).is_ok())
        .unwrap_or(false)
    {
        return true;
    }
    keychain_entry_exists()
}

#[cfg(target_os = "macos")]
fn keychain_entry_exists() -> bool {
    std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            &keychain_account(),
            "-s",
            KEYCHAIN_SERVICE,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn keychain_entry_exists() -> bool {
    false
}

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_CREDENTIAL_BYTES: u64 = 256 * 1024;

struct Auth {
    access_token: String,
    plan: Option<String>,
}

fn config_directory() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
}

fn credentials_path() -> Option<PathBuf> {
    config_directory().map(|directory| directory.join(".credentials.json"))
}

/// Where the CLI records its transcripts, one directory per project. Watched for write activity
/// only; never read.
fn projects_path() -> Option<PathBuf> {
    config_directory().map(|directory| directory.join("projects"))
}

fn keychain_account() -> String {
    std::env::var("USER")
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        })
        .or_else(|| {
            dirs::home_dir().and_then(|home| {
                home.file_name()
                    .map(|value| value.to_string_lossy().into_owned())
            })
        })
        .unwrap_or_else(|| "claude-code-user".into())
}

async fn read_keychain_credentials() -> Result<String, &'static str> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("Claude Code keychain is only available on macOS.");
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/security");
        command
            .args([
                "find-generic-password",
                "-a",
                &keychain_account(),
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let output = timeout(Duration::from_secs(8), command.output())
            .await
            .map_err(|_| "Claude Code keychain access timed out.")?
            .map_err(|_| "Claude Code keychain could not be opened.")?;
        if !output.status.success() {
            return Err("Claude Code login was not found in macOS Keychain.");
        }
        if output.stdout.len() as u64 > MAX_CREDENTIAL_BYTES {
            return Err("Claude Code login data is unavailable.");
        }
        String::from_utf8(output.stdout).map_err(|_| "Claude Code login format has changed.")
    }
}

fn read_file_credentials() -> Result<String, &'static str> {
    let path = credentials_path().ok_or("Claude Code login was not found.")?;
    let metadata = fs::metadata(&path).map_err(|_| "Please sign in to Claude Code first.")?;
    if !metadata.is_file() || metadata.len() > MAX_CREDENTIAL_BYTES {
        return Err("Claude Code login data is unavailable.");
    }
    fs::read_to_string(path).map_err(|_| "Please sign in to Claude Code first.")
}

fn parse_auth(raw: &str) -> Result<Auth, &'static str> {
    let value: Value =
        serde_json::from_str(raw.trim()).map_err(|_| "Claude Code login format has changed.")?;
    let oauth = value
        .get("claudeAiOauth")
        .or_else(|| value.get("claude_ai_oauth"))
        .unwrap_or(&value);
    let access_token = oauth
        .get("accessToken")
        .or_else(|| oauth.get("access_token"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Claude Code login expired. Please sign in again.")?
        .to_owned();
    let plan = oauth
        .get("subscriptionType")
        .or_else(|| oauth.get("subscription_type"))
        .and_then(Value::as_str)
        .map(format_plan);
    Ok(Auth { access_token, plan })
}

fn format_plan(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("max") {
        "MAX".into()
    } else if lower.contains("pro") {
        "PRO".into()
    } else if lower.contains("team") {
        "TEAM".into()
    } else if lower.contains("enterprise") {
        "ENTERPRISE".into()
    } else {
        value.replace('_', " ").to_uppercase()
    }
}

async fn load_auth() -> Result<Auth, &'static str> {
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(Auth {
                access_token: token,
                plan: None,
            });
        }
    }

    match read_keychain_credentials().await {
        Ok(raw) => parse_auth(&raw),
        Err(keychain_error) => match read_file_credentials() {
            Ok(raw) => parse_auth(&raw),
            Err(_) => Err(keychain_error),
        },
    }
}

fn headers(auth: &Auth) -> Result<HeaderMap, &'static str> {
    let mut result = HeaderMap::new();
    let mut bearer = HeaderValue::from_str(&format!("Bearer {}", auth.access_token))
        .map_err(|_| "Claude Code login data is invalid.")?;
    bearer.set_sensitive(true);
    result.insert(AUTHORIZATION, bearer);
    result.insert(ACCEPT, HeaderValue::from_static("application/json"));
    result.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    result.insert(
        "anthropic-beta",
        HeaderValue::from_static("oauth-2025-04-20"),
    );
    result.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    result.insert("anthropic-client-platform", HeaderValue::from_static("cli"));
    Ok(result)
}

fn parse_window(value: &Value, key: &str, window_seconds: u64) -> Option<UsageWindow> {
    let window = value.get(key)?;
    let used_percent = window
        .get("utilization")
        .or_else(|| window.get("used_percentage"))
        .and_then(Value::as_f64)?;
    let resets_at = window
        .get("resets_at")
        .or_else(|| window.get("resetsAt"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(UsageWindow {
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        resets_at,
        window_seconds,
    })
}

/// Extracts the per-model weekly buckets from the `limits` array.
///
/// `percent` here is the *used* share, matching `utilization` elsewhere in the payload, so it is
/// inverted to stay consistent with every other figure the app displays. Anything that is not a
/// `weekly_scoped` entry is left alone: the session and account-wide weekly buckets are already
/// read from the dedicated `five_hour` / `seven_day` objects.
fn parse_scoped_windows(usage: &Value) -> Vec<ScopedWindow> {
    let Some(limits) = usage.get("limits").and_then(Value::as_array) else {
        return Vec::new();
    };
    limits
        .iter()
        .filter_map(|limit| {
            if limit.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
                return None;
            }
            let used_percent = limit.get("percent").and_then(Value::as_f64)?;
            let label = limit
                .get("scope")?
                .get("model")?
                .get("display_name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())?
                .to_owned();
            Some(ScopedWindow {
                label,
                remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
                resets_at: limit
                    .get("resets_at")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn safe_http_failure(status: reqwest::StatusCode) -> (&'static str, &'static str) {
    match status.as_u16() {
        401 | 403 => (
            "signed_out",
            "Claude Code login expired. Open Claude Code and sign in again.",
        ),
        429 => (
            "unavailable",
            "Claude usage service is rate limited. It will retry automatically.",
        ),
        _ => (
            "unavailable",
            "Claude usage service is temporarily unavailable.",
        ),
    }
}

async fn limited_json(mut response: reqwest::Response) -> Result<Value, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_RESPONSE_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

pub async fn fetch_snapshot(client: &reqwest::Client) -> ProviderSnapshot {
    let auth = match load_auth().await {
        Ok(value) => value,
        Err(message) => {
            return ProviderSnapshot::failure_for("claude", "CLAUDE", "signed_out", message)
        }
    };
    let request_headers = match headers(&auth) {
        Ok(value) => value,
        Err(message) => {
            return ProviderSnapshot::failure_for("claude", "CLAUDE", "signed_out", message)
        }
    };
    let response = match client.get(USAGE_URL).headers(request_headers).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            let (status, message) = safe_http_failure(response.status());
            return ProviderSnapshot::failure_for("claude", "CLAUDE", status, message);
        }
        Err(_) => {
            return ProviderSnapshot::failure_for(
                "claude",
                "CLAUDE",
                "unavailable",
                "Network unavailable. It will retry automatically.",
            )
        }
    };
    let usage = match limited_json(response).await {
        Ok(value) => value,
        Err(_) => {
            return ProviderSnapshot::failure_for(
                "claude",
                "CLAUDE",
                "unavailable",
                "Claude usage response format has changed.",
            )
        }
    };
    let short_window = parse_window(&usage, "five_hour", 18_000);
    let weekly_window = parse_window(&usage, "seven_day", 604_800);
    if short_window.is_none() && weekly_window.is_none() {
        return ProviderSnapshot::failure_for(
            "claude",
            "CLAUDE",
            "unavailable",
            "Claude usage response does not include a supported usage window.",
        );
    }
    ProviderSnapshot {
        provider: "claude".into(),
        display_name: "CLAUDE".into(),
        plan: auth.plan,
        short_window,
        weekly_window,
        scoped_windows: parse_scoped_windows(&usage),
        reset_credits: None,
        reset_credit_expires_at: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: "ok".into(),
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_windows_as_remaining_percent() {
        let usage = serde_json::json!({
            "five_hour": {
                "utilization": 6.0,
                "resets_at": "2026-07-16T22:00:00Z"
            },
            "seven_day": {
                "utilization": 14.0,
                "resets_at": "2026-07-21T03:00:00Z"
            }
        });
        assert_eq!(
            parse_window(&usage, "five_hour", 18_000)
                .unwrap()
                .remaining_percent,
            94.0
        );
        assert_eq!(
            parse_window(&usage, "seven_day", 604_800)
                .unwrap()
                .remaining_percent,
            86.0
        );
    }

    /// Mirrors a real `/oauth/usage` payload: the per-model bucket lives in `limits`, not in the
    /// `seven_day_*` fields, which stay null even on an account that has one.
    #[test]
    fn reads_per_model_buckets_from_the_limits_array() {
        let usage = serde_json::json!({
            "seven_day_opus": null,
            "limits": [
                {"kind": "session", "percent": 27, "scope": null},
                {"kind": "weekly_all", "percent": 21, "scope": null},
                {
                    "kind": "weekly_scoped",
                    "percent": 25,
                    "resets_at": "2026-07-21T03:00:00Z",
                    "scope": {"model": {"id": null, "display_name": "Fable"}}
                }
            ]
        });

        let scoped = parse_scoped_windows(&usage);
        assert_eq!(
            scoped.len(),
            1,
            "session and weekly_all must not be duplicated"
        );
        assert_eq!(scoped[0].label, "Fable");
        // `percent` is the used share; every figure the app shows is remaining.
        assert_eq!(scoped[0].remaining_percent, 75.0);
        assert_eq!(scoped[0].resets_at.as_deref(), Some("2026-07-21T03:00:00Z"));
    }

    #[test]
    fn tolerates_a_payload_without_per_model_buckets() {
        assert!(parse_scoped_windows(&serde_json::json!({})).is_empty());
        assert!(parse_scoped_windows(&serde_json::json!({"limits": []})).is_empty());
        // An unnamed model cannot be labelled, so it is skipped rather than shown as blank.
        let unnamed = serde_json::json!({
            "limits": [{"kind": "weekly_scoped", "percent": 10, "scope": {"model": {"display_name": ""}}}]
        });
        assert!(parse_scoped_windows(&unnamed).is_empty());
    }

    #[test]
    fn formats_subscription_names() {
        assert_eq!(format_plan("default_claude_max_5x"), "MAX");
        assert_eq!(format_plan("pro"), "PRO");
    }
}
