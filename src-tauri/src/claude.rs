use std::{fs, path::PathBuf, process::Stdio, time::Duration};

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::{process::Command, time::timeout};

use crate::models::{ProviderSnapshot, UsageWindow};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_CREDENTIAL_BYTES: u64 = 256 * 1024;

struct Auth {
    access_token: String,
    plan: Option<String>,
}

fn credentials_path() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .map(|directory| directory.join(".credentials.json"))
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

    #[test]
    fn formats_subscription_names() {
        assert_eq!(format_plan("default_claude_max_5x"), "MAX");
        assert_eq!(format_plan("pro"), "PRO");
    }
}
