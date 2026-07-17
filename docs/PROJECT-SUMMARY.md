# CC project summary

CC 0.3.10 is a Tauri 2 macOS menu bar utility that reads existing local Codex Desktop and Claude Code sign-in state and queries each provider's quota service.

## Product behavior

- The menu bar always reserves a cool-blue Codex capsule on the left and a warm-orange Claude capsule on the right.
- The exact percentage stays visually centered. Five small bottom dots represent remaining started hours only for a 5-hour window; weekly fallback has no dots.
- The optional floating trigger is `100 × 100`. Clicking opens a `320 × 320` two-provider panel below it, or above when space requires, while keeping the trigger anchored.
- The compact trigger follows the frontmost app: Codex maps to Codex; other apps map to Claude; focusing CC keeps the previous provider.
- A present 5-hour window always wins, including a real 0%. Weekly quota is used only when the 5-hour window is absent.
- Transient provider failures retain the last good value as stale data in both the tray and floating window. A signed-out state clears old values.

## Safety boundary

- Credentials are read only for the quota request and are never copied into preferences.
- Responses are limited to 1 MB, auth files to 256 KB, and HTTP redirects are disabled.
- CC includes no telemetry, prompt collection, account mutation, or reset-credit redemption.

See `README.md`, `PRIVACY.md`, and `SECURITY.md` for public documentation.
