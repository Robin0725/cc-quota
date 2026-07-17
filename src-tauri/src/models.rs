use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub remaining_percent: f64,
    pub resets_at: Option<String>,
    pub window_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub provider: String,
    pub display_name: String,
    pub plan: Option<String>,
    pub short_window: Option<UsageWindow>,
    pub weekly_window: Option<UsageWindow>,
    pub reset_credits: Option<u64>,
    pub reset_credit_expires_at: Vec<String>,
    pub updated_at: String,
    pub status: String,
    pub message: Option<String>,
}

impl ProviderSnapshot {
    pub fn failure_for(provider: &str, display_name: &str, status: &str, message: &str) -> Self {
        Self {
            provider: provider.into(),
            display_name: display_name.into(),
            plan: None,
            short_window: None,
            weekly_window: None,
            reset_credits: None,
            reset_credit_expires_at: Vec::new(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            status: status.into(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPreferences {
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
    #[serde(default)]
    pub widget_visible: bool,
    #[serde(default)]
    pub pinned_provider: Option<String>,
    #[serde(default = "default_auto_rotate_seconds")]
    pub auto_rotate_seconds: u64,
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_always_on_top() -> bool {
    true
}
fn default_language() -> String {
    "zh-CN".into()
}
fn default_auto_rotate_seconds() -> u64 {
    12
}

impl Default for WidgetPreferences {
    fn default() -> Self {
        Self {
            locked: false,
            always_on_top: true,
            widget_visible: false,
            pinned_provider: None,
            auto_rotate_seconds: default_auto_rotate_seconds(),
            language: default_language(),
        }
    }
}

impl WidgetPreferences {
    pub fn normalized(mut self) -> Self {
        self.auto_rotate_seconds = self.auto_rotate_seconds.clamp(5, 300);
        if !matches!(
            self.pinned_provider.as_deref(),
            Some("codex") | Some("claude")
        ) {
            self.pinned_provider = None;
        }
        if self.language != "en" && self.language != "zh-CN" {
            self.language = default_language();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::WidgetPreferences;

    #[test]
    fn missing_preference_fields_use_safe_defaults() {
        let value: WidgetPreferences = serde_json::from_str(r#"{"widgetVisible":true}"#).unwrap();
        assert!(!value.locked);
        assert!(value.always_on_top);
        assert!(value.widget_visible);
        assert!(value.pinned_provider.is_none());
        assert_eq!(value.auto_rotate_seconds, 12);
        assert_eq!(value.language, "zh-CN");
    }
}
