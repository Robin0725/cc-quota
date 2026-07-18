//! Localized strings for the menu bar.
//!
//! The widget renders its own copy from `src/lib/i18n.ts`; this is the tray's equivalent so both
//! surfaces follow the same language preference instead of the menu being permanently Chinese.
//!
//! Provider failure text (`ProviderSnapshot::message`) still arrives from the API layer as English
//! prose and is passed through untranslated. Localizing it requires the snapshot contract to carry
//! reason codes rather than sentences, which is a separate change.

pub struct TrayCopy {
    pub show_widget: &'static str,
    pub always_on_top: &'static str,
    pub disable_click_through: &'static str,
    pub refresh_now: &'static str,
    /// Names the language being switched *to*, so the item reads as an action.
    pub switch_language: &'static str,
    pub launch_at_login: &'static str,
    pub quit: &'static str,
    pub loading: &'static str,
    pub stale_suffix: &'static str,
    pub reset_unknown: &'static str,
    /// `chrono` format string; only `%` sequences are substituted, the rest is literal.
    pub reset_format: &'static str,
    pub unavailable: &'static str,
}

const ZH_CN: TrayCopy = TrayCopy {
    show_widget: "显示悬浮窗",
    always_on_top: "悬浮窗始终置顶",
    disable_click_through: "取消鼠标穿透",
    refresh_now: "立即刷新",
    switch_language: "Switch to English",
    launch_at_login: "登录时启动",
    quit: "退出 CC Quota",
    loading: "正在读取…",
    stale_suffix: " · 旧数据",
    reset_unknown: "重置时间未知",
    reset_format: "%m/%d %H:%M 重置",
    unavailable: "暂时无法读取额度",
};

const EN: TrayCopy = TrayCopy {
    show_widget: "Show floating widget",
    always_on_top: "Keep widget on top",
    disable_click_through: "Disable click-through",
    refresh_now: "Refresh now",
    switch_language: "切换为中文",
    launch_at_login: "Launch at login",
    quit: "Quit CC Quota",
    loading: "Reading…",
    stale_suffix: " · Stale",
    reset_unknown: "Reset time unknown",
    reset_format: "resets %m/%d %H:%M",
    unavailable: "Quota temporarily unavailable",
};

/// Falls back to Chinese for anything unrecognized, matching `WidgetPreferences::normalized`.
pub fn tray_copy(language: &str) -> &'static TrayCopy {
    if language == "en" {
        &EN
    } else {
        &ZH_CN
    }
}

#[cfg(test)]
mod tests {
    use super::tray_copy;

    #[test]
    fn selects_english_only_for_the_english_preference() {
        assert_eq!(tray_copy("en").quit, "Quit CC Quota");
        assert_eq!(tray_copy("zh-CN").quit, "退出 CC Quota");
        assert_eq!(tray_copy("fr").quit, "退出 CC Quota");
    }

    #[test]
    fn language_item_names_the_target_language() {
        // Reading the item as an action is what makes the toggle discoverable.
        assert_eq!(tray_copy("zh-CN").switch_language, "Switch to English");
        assert_eq!(tray_copy("en").switch_language, "切换为中文");
    }
}
