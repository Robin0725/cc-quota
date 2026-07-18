//! Provider registry.
//!
//! Adding an AI provider is meant to cost one descriptor, one adapter and one line in
//! [`REGISTRY`]. Nothing outside this module may branch on a provider id.

pub mod claude;
pub mod codex;

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::models::ProviderSnapshot;

/// Tray capsule colours. Declared here so the bitmap and the frontend accent read from one source.
#[derive(Clone, Copy)]
pub struct CapsulePalette {
    pub border: [u8; 4],
    pub track: [u8; 4],
    pub fill_top: [u8; 4],
    pub fill_bottom: [u8; 4],
}

/// Declarative description of a provider. Everything here is static data; behaviour lives in
/// [`ProviderAdapter`].
pub struct ProviderDescriptor {
    /// Stable id shared by the backend, the frontend and the preferences file.
    /// Published values must never change.
    pub id: &'static str,
    pub display_name: &'static str,
    /// Two letter badge drawn on the menu bar capsule.
    pub abbreviation: &'static str,
    pub palette: CapsulePalette,
    /// Frontend accent colour. Must be the hex form of `palette.fill_bottom`.
    pub accent_hex: &'static str,
    /// Foreground app attribution: a bundle id or process name containing any of these substrings
    /// belongs to this provider. All lowercase; callers lowercase their input.
    pub focus_hints: &'static [&'static str],
}

#[async_trait::async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> &'static ProviderDescriptor;

    /// Whether a local login exists (credential file / keychain entry).
    /// Existence check only: no network requests, no decrypting or echoing credentials.
    fn is_configured(&self) -> bool;

    /// Reads the quota. Never panics, never returns `Err`: failures come back as
    /// `ProviderSnapshot::failure_for(..)` with status `signed_out` or `unavailable`.
    async fn fetch_snapshot(&self, client: &reqwest::Client) -> ProviderSnapshot;
}

static CODEX: codex::CodexAdapter = codex::CodexAdapter;
static CLAUDE: claude::ClaudeAdapter = claude::ClaudeAdapter;

/// Every known provider. Order is the UI order.
static REGISTRY: &[&(dyn ProviderAdapter + 'static)] = &[&CODEX, &CLAUDE];

pub fn all() -> &'static [&'static dyn ProviderAdapter] {
    REGISTRY
}

pub fn find(id: &str) -> Option<&'static dyn ProviderAdapter> {
    all()
        .iter()
        .copied()
        .find(|item| item.descriptor().id == id)
}

pub fn is_known(id: &str) -> bool {
    find(id).is_some()
}

/// Provider the foreground app is attributed to when no `focus_hints` match.
pub const DEFAULT_FOCUS_PROVIDER: &str = "claude";

/// `is_configured` may touch the filesystem or spawn `security`, and the frontmost-app poll asks
/// for it several times a second, so the answer is memoised briefly.
const CONFIGURED_TTL: Duration = Duration::from_secs(30);

fn configured_ids() -> Vec<&'static str> {
    static CACHE: Mutex<Option<(Instant, Vec<&'static str>)>> = Mutex::new(None);

    if let Ok(cache) = CACHE.lock() {
        if let Some((stamp, ids)) = cache.as_ref() {
            if stamp.elapsed() < CONFIGURED_TTL {
                return ids.clone();
            }
        }
    }
    let ids: Vec<&'static str> = all()
        .iter()
        .filter(|item| item.is_configured())
        .map(|item| item.descriptor().id)
        .collect();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), ids.clone()));
    }
    ids
}

/// Providers with a local login, in registry order.
pub fn configured() -> Vec<&'static dyn ProviderAdapter> {
    let ids = configured_ids();
    all()
        .iter()
        .copied()
        .filter(|item| ids.contains(&item.descriptor().id))
        .collect()
}

/// Providers the app should show. Same as [`configured`], except that a machine with no login at
/// all still gets the full list so the menu bar shows "signed out" capsules instead of nothing.
pub fn active() -> Vec<&'static dyn ProviderAdapter> {
    let found = configured();
    if found.is_empty() {
        all().to_vec()
    } else {
        found
    }
}

/// Provider that owns the frontmost app, or `None` when it is this app itself.
pub fn classify_focus(
    bundle_id: Option<&str>,
    name: Option<&str>,
    default_provider: &'static str,
) -> Option<&'static str> {
    let bundle_id = bundle_id.unwrap_or_default().to_ascii_lowercase();
    let name = name.unwrap_or_default().to_ascii_lowercase();
    // The bundle id is the reliable check; the name is a fallback for when AppKit reports no
    // identifier. It must track the product name, or focusing the app would count as "some other
    // app" and flip the orb to the default provider — the flicker this guard exists to prevent.
    if bundle_id == "app.ccquota.desktop" || name == "cc" || name == "cc quota" {
        return None;
    }
    for adapter in all() {
        let descriptor = adapter.descriptor();
        if descriptor
            .focus_hints
            .iter()
            .any(|hint| bundle_id.contains(hint) || name.contains(hint))
        {
            return Some(descriptor.id);
        }
    }
    Some(default_provider)
}

/// Falls back to the first configured provider when the default one is not signed in.
pub fn default_focus_provider() -> &'static str {
    let found = active();
    found
        .iter()
        .find(|item| item.descriptor().id == DEFAULT_FOCUS_PROVIDER)
        .or_else(|| found.first())
        .map(|item| item.descriptor().id)
        .unwrap_or(DEFAULT_FOCUS_PROVIDER)
}

/// Shape consumed by the frontend so colours and abbreviations are defined once, in Rust.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptorDto {
    pub id: String,
    pub display_name: String,
    pub abbreviation: String,
    pub accent_hex: String,
}

impl From<&'static ProviderDescriptor> for ProviderDescriptorDto {
    fn from(value: &'static ProviderDescriptor) -> Self {
        Self {
            id: value.id.into(),
            display_name: value.display_name.into(),
            abbreviation: value.abbreviation.into(),
            accent_hex: value.accent_hex.into(),
        }
    }
}

pub fn descriptor_dtos() -> Vec<ProviderDescriptorDto> {
    active()
        .into_iter()
        .map(|item| ProviderDescriptorDto::from(item.descriptor()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(color: [u8; 4]) -> String {
        format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
    }

    #[test]
    fn registry_ids_are_unique_and_lowercase() {
        let mut seen = Vec::new();
        for adapter in all() {
            let descriptor = adapter.descriptor();
            assert!(
                !seen.contains(&descriptor.id),
                "duplicate provider id: {}",
                descriptor.id
            );
            assert_eq!(descriptor.id, descriptor.id.to_ascii_lowercase());
            assert_eq!(descriptor.abbreviation.chars().count(), 2);
            seen.push(descriptor.id);
        }
        assert!(!seen.is_empty());
    }

    /// The accent the frontend paints with must be the colour the tray capsule fills with, or the
    /// two drift apart the moment a palette is tweaked.
    #[test]
    fn accent_hex_matches_the_capsule_fill() {
        for adapter in all() {
            let descriptor = adapter.descriptor();
            assert_eq!(
                descriptor.accent_hex,
                hex(descriptor.palette.fill_bottom),
                "accent drift on {}",
                descriptor.id
            );
        }
    }

    #[test]
    fn focus_hints_are_lowercase_and_non_empty() {
        for adapter in all() {
            let descriptor = adapter.descriptor();
            assert!(!descriptor.focus_hints.is_empty(), "{}", descriptor.id);
            for hint in descriptor.focus_hints {
                assert_eq!(*hint, hint.to_ascii_lowercase());
            }
        }
    }

    /// The frontend reads these keys verbatim; renaming one silently blanks the UI.
    #[test]
    fn descriptor_dto_serializes_as_camel_case() {
        let dto = ProviderDescriptorDto::from(&codex::DESCRIPTOR);
        let value: serde_json::Value = serde_json::to_value(&dto).unwrap();
        let object = value.as_object().unwrap();
        let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["abbreviation", "accentHex", "displayName", "id"]);
        assert_eq!(object["id"], "codex");
        assert_eq!(object["displayName"], "Codex");
        assert_eq!(object["abbreviation"], "CX");
        assert_eq!(object["accentHex"], "#2f6fed");
    }

    #[test]
    fn default_focus_provider_is_registered() {
        assert!(is_known(DEFAULT_FOCUS_PROVIDER));
    }

    #[test]
    fn maps_a_hinted_bundle_to_its_provider_and_everything_else_to_the_default() {
        assert_eq!(
            classify_focus(Some("com.openai.codex"), Some("ChatGPT"), "claude"),
            Some("codex")
        );
        assert_eq!(
            classify_focus(
                Some("com.anthropic.claudefordesktop"),
                Some("Claude"),
                "claude"
            ),
            Some("claude")
        );
        assert_eq!(
            classify_focus(Some("com.apple.finder"), Some("Finder"), "claude"),
            Some("claude")
        );
        // Every registered provider must be reachable through at least one of its own hints.
        for adapter in all() {
            let descriptor = adapter.descriptor();
            let hint = descriptor.focus_hints[0];
            assert_eq!(
                classify_focus(Some(&format!("com.example.{hint}")), None, "claude"),
                Some(descriptor.id),
                "hint {hint} did not resolve to {}",
                descriptor.id
            );
        }
    }

    #[test]
    fn ignores_cc_itself_to_avoid_focus_flicker() {
        assert_eq!(
            classify_focus(Some("app.ccquota.desktop"), Some("CC Quota"), "claude"),
            None
        );
        // Falls back to the product name when AppKit reports no bundle identifier.
        assert_eq!(classify_focus(None, Some("CC Quota"), "claude"), None);
        assert_eq!(classify_focus(None, Some("CC"), "claude"), None);
    }
}
