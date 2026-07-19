use std::{
    fs,
    path::{Path, PathBuf},
};

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use serde_json::Value;

use crate::models::{ProviderSnapshot, UsageWindow};
use crate::providers::{CapsulePalette, ProviderAdapter, ProviderDescriptor};

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    id: "kimicode",
    display_name: "Kimi Code",
    abbreviation: "KM",
    palette: CapsulePalette {
        // Codex and Claude darken their accent to about half luminance for the border and track;
        // the same halving keeps the third capsule in the same visual family.
        border: [62, 46, 107, 255],
        track: [62, 46, 107, 255],
        fill_top: [124, 92, 214, 255],
        fill_bottom: [124, 92, 214, 255],
    },
    accent_hex: "#7c5cd6",
    focus_hints: &["kimi"],
};

pub struct KimiCodeAdapter;

#[async_trait::async_trait]
impl ProviderAdapter for KimiCodeAdapter {
    fn descriptor(&self) -> &'static ProviderDescriptor {
        &DESCRIPTOR
    }

    /// Local existence check only: the file is parsed for the presence of an access token and the
    /// value is dropped without ever leaving this function.
    fn is_configured(&self) -> bool {
        credentials_path().is_some_and(|path| load_credentials(&path).is_ok())
    }

    fn activity_paths(&self) -> Vec<PathBuf> {
        user_history_path().into_iter().collect()
    }

    async fn fetch_snapshot(&self, client: &reqwest::Client) -> ProviderSnapshot {
        fetch_snapshot(client).await
    }
}

const USAGE_URL: &str = "https://api.kimi.com/coding/v1/usages";
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_CREDENTIAL_BYTES: u64 = 256 * 1024;
const SHORT_WINDOW_SECONDS: u64 = 18_000;
const WEEKLY_WINDOW_SECONDS: u64 = 604_800;
/// Treat a token as spent slightly before its stated expiry so one cannot lapse mid-request.
const EXPIRY_MARGIN_SECONDS: i64 = 60;
const DISPLAY_NAME: &str = "KIMI CODE";

/// Only the access token and its expiry are read. The refresh token in the same file is
/// deliberately ignored — see [`access_token`].
struct Credentials {
    access_token: String,
    expires_at: Option<i64>,
}

fn kimi_home() -> Option<PathBuf> {
    std::env::var_os("KIMI_CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".kimi-code")))
}

fn credentials_path() -> Option<PathBuf> {
    kimi_home().map(|home| home.join("credentials").join("kimi-code.json"))
}

/// Written only when the user submits a prompt. The sessions directory next to it is the wrong
/// activity signal: a Kimi agent grinding through a long task in the background streams into it
/// every minute or two, and the widget would follow that agent instead of the person. Watched by
/// modification time only; never read.
fn user_history_path() -> Option<PathBuf> {
    kimi_home().map(|home| home.join("user-history"))
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .filter(|item| !item.is_empty())
}

/// Kimi reports every quota number as a JSON *string* (`"limit": "100"`). Older builds and the
/// public docs use plain numbers, so both shapes are accepted rather than one being assumed.
fn number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        let item = value.get(*key)?;
        item.as_f64()
            .or_else(|| item.as_str()?.trim().parse::<f64>().ok())
    })
}

fn integer(value: &Value, keys: &[&str]) -> Option<i64> {
    number(value, keys).map(|item| item as i64)
}

fn timestamp(value: &Value) -> Option<String> {
    // The live response uses `resetTime`; the aliases cover the shapes the docs describe.
    text(
        value,
        &[
            "resetTime",
            "reset_time",
            "resetAt",
            "reset_at",
            "resetsAt",
            "resets_at",
        ],
    )
    .map(str::to_owned)
}

/// `"TIME_UNIT_MINUTE"` — the enum name arrives with its protobuf prefix attached, so a bare
/// `"MINUTE"` comparison silently yields a zero-length window and no five hour bucket is ever
/// recognised.
fn unit_seconds(unit: &str) -> Option<u64> {
    let unit = unit.strip_prefix("TIME_UNIT_").unwrap_or(unit);
    match unit.to_ascii_uppercase().trim_end_matches('S') {
        "SECOND" => Some(1),
        "MINUTE" => Some(60),
        "HOUR" => Some(3_600),
        "DAY" => Some(86_400),
        "WEEK" => Some(604_800),
        _ => None,
    }
}

fn window_seconds(window: &Value) -> u64 {
    let duration = number(window, &["duration"]).unwrap_or(0.0).max(0.0);
    let unit = text(window, &["timeUnit", "time_unit"]).unwrap_or("TIME_UNIT_SECOND");
    unit_seconds(unit)
        .map(|factor| (duration * factor as f64) as u64)
        .unwrap_or(0)
}

/// The top level `usage` object reports `remaining`, not `used`; deriving the percentage from a
/// `used` field that is not there would report a permanently full quota.
fn remaining_percent(detail: &Value) -> Option<f64> {
    let limit = number(detail, &["limit", "total"])?;
    if limit <= 0.0 {
        return None;
    }
    let remaining = number(detail, &["remaining", "left"])
        .or_else(|| number(detail, &["used", "usage"]).map(|used| limit - used))?;
    Some((remaining / limit * 100.0).clamp(0.0, 100.0))
}

fn parse_window(detail: &Value, seconds: u64) -> Option<UsageWindow> {
    Some(UsageWindow {
        remaining_percent: remaining_percent(detail)?,
        resets_at: timestamp(detail),
        window_seconds: seconds,
    })
}

fn find_short_window(payload: &Value) -> Option<UsageWindow> {
    let limits = payload.get("limits")?.as_array()?;
    limits.iter().find_map(|entry| {
        let seconds = entry.get("window").map(window_seconds).unwrap_or(0);
        if seconds.abs_diff(SHORT_WINDOW_SECONDS) > 60 {
            return None;
        }
        let detail = entry.get("detail").unwrap_or(entry);
        parse_window(detail, seconds)
    })
}

fn format_plan(value: &str) -> String {
    value
        .strip_prefix("LEVEL_")
        .unwrap_or(value)
        .replace('_', " ")
        .to_uppercase()
}

/// Builds the snapshot from a decoded usage payload. Kept separate from the request so the field
/// quirks can be covered without touching the network.
fn build_snapshot(payload: &Value) -> Result<ProviderSnapshot, &'static str> {
    let short_window = find_short_window(payload);
    // `boosterWallet` (a paid overage pack) is absent on accounts without one; it is read as an
    // optional extra and never gates the snapshot.
    let weekly_window = payload
        .get("usage")
        .and_then(|usage| parse_window(usage, WEEKLY_WINDOW_SECONDS));
    if short_window.is_none() && weekly_window.is_none() {
        return Err("Quota response does not include a supported usage window.");
    }

    let plan = payload
        .get("user")
        .and_then(|user| user.get("membership"))
        .and_then(|membership| text(membership, &["level"]))
        .map(format_plan);

    Ok(ProviderSnapshot {
        provider: DESCRIPTOR.id.into(),
        display_name: DISPLAY_NAME.into(),
        plan,
        short_window,
        weekly_window,
        // Kimi reports no per-model buckets.
        scoped_windows: Vec::new(),
        reset_credits: None,
        reset_credit_expires_at: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        status: "ok".into(),
        message: None,
    })
}

fn parse_credentials(raw: &str) -> Result<Credentials, &'static str> {
    let value: Value =
        serde_json::from_str(raw.trim()).map_err(|_| "Kimi Code login format has changed.")?;
    let access_token = text(&value, &["access_token", "accessToken"])
        .ok_or("Kimi Code login expired. Please sign in again.")?
        .to_owned();
    Ok(Credentials {
        access_token,
        expires_at: integer(&value, &["expires_at", "expiresAt"]),
    })
}

fn load_credentials(path: &Path) -> Result<Credentials, &'static str> {
    let metadata = fs::metadata(path).map_err(|_| "Please sign in to Kimi Code first.")?;
    if !metadata.is_file() || metadata.len() > MAX_CREDENTIAL_BYTES {
        return Err("Kimi Code login data is unavailable.");
    }
    let raw = fs::read_to_string(path).map_err(|_| "Please sign in to Kimi Code first.")?;
    parse_credentials(&raw)
}

/// Some CLIs persist the expiry in milliseconds; a millisecond value read as seconds would look
/// like the year 57000 and the token would never be refreshed.
fn expiry_seconds(value: i64) -> i64 {
    if value.abs() > 100_000_000_000 {
        value / 1000
    } else {
        value
    }
}

fn is_expired(expires_at: Option<i64>, now: i64) -> bool {
    expires_at.is_some_and(|value| expiry_seconds(value) - EXPIRY_MARGIN_SECONDS <= now)
}

/// Reads the token the CLI last wrote. **This app never refreshes it**, on purpose:
///
/// * A Kimi access token lives fifteen minutes and the CLI renews it while the user works, so the
///   file is current exactly when the quota number matters.
/// * If the server rotates refresh tokens on use, refreshing here without writing the file back
///   would invalidate the user's CLI login — and refreshing *and* writing would race the CLI for
///   ownership of its own credentials. Neither risk buys anything beyond a live number for a user
///   who is not using Kimi right now.
///
/// An expired token therefore takes the ordinary failure path; the last good reading is what the
/// UI keeps showing until the CLI writes a fresh token.
fn access_token(now: i64) -> Result<String, (&'static str, &'static str)> {
    let path = credentials_path().ok_or(("signed_out", "Kimi Code login was not found."))?;
    let credentials = load_credentials(&path).map_err(|message| ("signed_out", message))?;
    if is_expired(credentials.expires_at, now) {
        // Not a sign-out: the CLI renews this token itself, so the reading is merely stale.
        return Err((
            "unavailable",
            "Kimi Code token has expired. It renews the next time the CLI runs.",
        ));
    }
    Ok(credentials.access_token)
}

fn headers(token: &str) -> Result<HeaderMap, &'static str> {
    let mut result = HeaderMap::new();
    let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "Kimi Code login data is invalid.")?;
    bearer.set_sensitive(true);
    result.insert(AUTHORIZATION, bearer);
    result.insert(ACCEPT, HeaderValue::from_static("application/json"));
    Ok(result)
}

fn safe_http_failure(status: reqwest::StatusCode) -> (&'static str, &'static str) {
    match status.as_u16() {
        401 | 403 => (
            "signed_out",
            "Kimi Code login expired. Please sign in again.",
        ),
        429 => (
            "unavailable",
            "Quota service is rate limited. It will retry automatically.",
        ),
        _ => ("unavailable", "Quota service is temporarily unavailable."),
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

fn failure(status: &str, message: &str) -> ProviderSnapshot {
    ProviderSnapshot::failure_for(DESCRIPTOR.id, DISPLAY_NAME, status, message)
}

pub async fn fetch_snapshot(client: &reqwest::Client) -> ProviderSnapshot {
    let token = match access_token(chrono::Utc::now().timestamp()) {
        Ok(value) => value,
        Err((status, message)) => return failure(status, message),
    };
    let request_headers = match headers(&token) {
        Ok(value) => value,
        Err(message) => return failure("signed_out", message),
    };

    let response = match client.get(USAGE_URL).headers(request_headers).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            let (status, message) = safe_http_failure(response.status());
            return failure(status, message);
        }
        Err(_) => {
            return failure(
                "unavailable",
                "Network unavailable. It will retry automatically.",
            )
        }
    };
    let payload = match limited_json(response).await {
        Ok(value) => value,
        Err(_) => return failure("unavailable", "Quota response format has changed."),
    };
    match build_snapshot(&payload) {
        Ok(snapshot) => snapshot,
        Err(message) => failure("unavailable", message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured response, redacted. Numbers are strings, the account level carries its
    /// `LEVEL_` prefix and there is no `boosterWallet`.
    fn live_payload() -> Value {
        serde_json::json!({
            "usage": { "limit": "100", "remaining": "100", "resetTime": "2026-07-25T16:06:49Z" },
            "limits": [ { "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                          "detail": { "limit": "100", "used": "1", "remaining": "99",
                                      "resetTime": "2026-07-18T21:06:49Z" } } ],
            "totalQuota": { "limit": "100", "remaining": "99" },
            "user": { "membership": { "level": "LEVEL_INTERMEDIATE" } },
            "subType": "TYPE_PURCHASE"
        })
    }

    #[test]
    fn reads_quota_numbers_written_as_strings() {
        let stringly = serde_json::json!({"limit": "100", "remaining": "25"});
        assert_eq!(remaining_percent(&stringly), Some(25.0));
        // Plain numbers, as the public docs show them, must keep working.
        let numeric = serde_json::json!({"limit": 100, "remaining": 25});
        assert_eq!(remaining_percent(&numeric), Some(25.0));
        assert_eq!(number(&stringly, &["missing", "limit"]), Some(100.0));
    }

    #[test]
    fn derives_the_remaining_share_from_either_direction() {
        // The live payload reports `remaining`, so treating it as `used` would invert the reading.
        let remaining = serde_json::json!({"limit": "200", "remaining": "50"});
        assert_eq!(remaining_percent(&remaining), Some(25.0));
        let used_only = serde_json::json!({"limit": "200", "used": "50"});
        assert_eq!(remaining_percent(&used_only), Some(75.0));
        // `remaining` wins when a payload carries both.
        let both = serde_json::json!({"limit": "200", "remaining": "50", "used": "150"});
        assert_eq!(remaining_percent(&both), Some(25.0));
        assert_eq!(remaining_percent(&serde_json::json!({"limit": "0"})), None);
    }

    #[test]
    fn strips_the_time_unit_enum_prefix() {
        assert_eq!(unit_seconds("TIME_UNIT_MINUTE"), Some(60));
        assert_eq!(unit_seconds("MINUTE"), Some(60));
        assert_eq!(unit_seconds("TIME_UNIT_HOUR"), Some(3_600));
        assert_eq!(unit_seconds("TIME_UNIT_UNSPECIFIED"), None);
        assert_eq!(
            window_seconds(&serde_json::json!({"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"})),
            18_000
        );
        // An unrecognised unit must not be silently counted as seconds.
        assert_eq!(
            window_seconds(
                &serde_json::json!({"duration": 300, "timeUnit": "TIME_UNIT_LIGHTYEAR"})
            ),
            0
        );
    }

    #[test]
    fn recognises_the_five_hour_bucket_from_duration_and_unit() {
        let window = find_short_window(&live_payload()).unwrap();
        assert_eq!(window.window_seconds, 18_000);
        assert_eq!(window.remaining_percent, 99.0);
        assert_eq!(window.resets_at.as_deref(), Some("2026-07-18T21:06:49Z"));
        // A daily bucket is not a five hour bucket.
        let daily = serde_json::json!({
            "limits": [{"window": {"duration": 1, "timeUnit": "TIME_UNIT_DAY"},
                        "detail": {"limit": "100", "remaining": "40"}}]
        });
        assert!(find_short_window(&daily).is_none());
    }

    #[test]
    fn maps_the_measured_payload_without_a_booster_wallet() {
        let snapshot = build_snapshot(&live_payload()).unwrap();
        assert_eq!(snapshot.provider, "kimicode");
        assert_eq!(snapshot.status, "ok");
        assert_eq!(snapshot.plan.as_deref(), Some("INTERMEDIATE"));
        assert_eq!(
            snapshot.short_window.as_ref().unwrap().remaining_percent,
            99.0
        );
        let weekly = snapshot.weekly_window.as_ref().unwrap();
        assert_eq!(weekly.remaining_percent, 100.0);
        assert_eq!(weekly.window_seconds, 604_800);
        assert!(snapshot.scoped_windows.is_empty());

        // The paid overage pack is optional: present or absent, the reading is the same.
        let mut with_booster = live_payload();
        with_booster["boosterWallet"] = serde_json::json!({"limit": "500", "remaining": "500"});
        let boosted = build_snapshot(&with_booster).unwrap();
        assert_eq!(boosted.short_window.unwrap().remaining_percent, 99.0);
    }

    #[test]
    fn reports_an_unsupported_payload_instead_of_a_confident_zero() {
        assert!(build_snapshot(&serde_json::json!({"subType": "TYPE_PURCHASE"})).is_err());
    }

    #[test]
    fn treats_a_missing_credential_file_as_signed_out() {
        let missing = std::env::temp_dir()
            .join("cc-quota-kimicode-absent")
            .join("credentials")
            .join("kimi-code.json");
        assert!(load_credentials(&missing).is_err());
        // A directory in the file's place must not read as a login either.
        assert!(load_credentials(&std::env::temp_dir()).is_err());
        assert!(parse_credentials("{\"token_type\":\"Bearer\"}").is_err());
        assert!(parse_credentials("not json").is_err());
    }

    #[test]
    fn counts_a_token_as_expired_at_or_near_its_stated_expiry() {
        let now = 1_800_000_000;
        assert!(!is_expired(Some(now + 3_600), now));
        assert!(is_expired(Some(now + 30), now));
        assert!(is_expired(Some(now - 1), now));
        // Milliseconds, as some CLIs write them, must not read as a distant future.
        assert!(is_expired(Some((now - 1) * 1000), now));
        assert!(!is_expired(Some((now + 3_600) * 1000), now));
        // No expiry recorded: use the token and let a 401 decide.
        assert!(!is_expired(None, now));
    }

    /// The app must not participate in Kimi's auth lifecycle: an expired token is reported as a
    /// plain failure, never traded in for a new one. Refreshing here could invalidate the user's
    /// CLI login if the server rotates refresh tokens, and the CLI renews the file itself anyway.
    #[test]
    fn an_expired_token_fails_instead_of_being_refreshed() {
        let directory = std::env::temp_dir().join("cc-quota-kimicode-expired");
        fs::create_dir_all(directory.join("credentials")).unwrap();
        let path = directory.join("credentials").join("kimi-code.json");
        // A live-shaped file: fifteen minute lifetime, refresh token present and to be ignored.
        fs::write(
            &path,
            r#"{"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":1800000900,"expires_in":900,"token_type":"Bearer"}"#,
        )
        .unwrap();
        std::env::set_var("KIMI_CODE_HOME", &directory);

        // Inside the lifetime the stored token is used as-is.
        assert_eq!(access_token(1_800_000_000).unwrap(), "fake-access");
        // Past it, the call fails rather than reaching for the refresh token.
        let (status, message) = access_token(1_800_001_000).unwrap_err();
        assert_eq!(status, "unavailable");
        assert!(!message.contains("fake"));

        std::env::remove_var("KIMI_CODE_HOME");
        let _ = fs::remove_dir_all(&directory);
    }

    /// The gateway in front of the usage endpoint answers with an HTML `504` under load, and has
    /// been measured holding a request open for twenty seconds. Neither may look like a reading:
    /// an expired login is the one failure worth naming, everything else is "come back later".
    #[test]
    fn maps_gateway_failures_to_a_retryable_status() {
        use reqwest::StatusCode;

        assert_eq!(safe_http_failure(StatusCode::UNAUTHORIZED).0, "signed_out");
        assert_eq!(safe_http_failure(StatusCode::FORBIDDEN).0, "signed_out");
        for status in [
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::TOO_MANY_REQUESTS,
        ] {
            assert_eq!(safe_http_failure(status).0, "unavailable", "{status}");
        }
        // The 504 body is nginx's HTML error page, not JSON. It must be rejected as unparsable
        // rather than read as an empty quota.
        assert!(build_snapshot(&serde_json::json!("<html>504 Gateway Time-out</html>")).is_err());
        assert!(serde_json::from_slice::<Value>(b"<html>504</html>").is_err());
    }

    #[test]
    fn parses_a_credential_file_without_echoing_it() {
        let credentials = parse_credentials(
            r#"{"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":1800000000,"token_type":"Bearer"}"#,
        )
        .unwrap();
        assert_eq!(credentials.access_token, "fake-access");
        assert_eq!(credentials.expires_at, Some(1_800_000_000));
    }
}
