# CC project summary

CC Quota 0.5.12 is a Tauri 2 macOS menu bar utility that reads the sign-in state Codex Desktop, Claude Code, and Kimi Code already keep locally, and queries each provider's quota service. Providers are registered rather than hardcoded, so support for another one is a descriptor and an adapter.

## Product behavior

- The menu bar renders one provider-coloured capsule for every signed-in provider in registry order.
- The exact percentage stays visually centered. Five small bottom dots represent remaining started hours only for a 5-hour window; weekly fallback has no dots.
- The optional floating trigger is `100 × 100`. Clicking opens a `320 × 320` provider panel below it, or above when space requires, while keeping the trigger anchored.
- Whenever a weekly window exists, the original 3px `border-left` accent rail reveals its remainder bottom-up on each detail card; the compact trigger borrows the same rail geometry without turning its focus outline into data. Hovering the trigger reveals the exact weekly figure, and weekly-only providers keep the visual edge even though the central number already carries the same value.
- The compact trigger follows the focused provider window when identifiable, otherwise the last assistant the user typed to; focusing CC keeps the previous provider.
- A present 5-hour window always wins, including a real 0%. Weekly quota is used only when the 5-hour window is absent.
- Transient provider failures retain the last good value as stale data in both the tray and floating window. A signed-out state clears old values.

## Safety boundary

- Credentials are read only for the quota request and are never copied into preferences.
- Responses are limited to 1 MB, auth files to 256 KB, and HTTP redirects are disabled.
- CC includes no telemetry, prompt collection, account mutation, or reset-credit redemption.

See `README.md`, `PRIVACY.md`, and `SECURITY.md` for public documentation.
