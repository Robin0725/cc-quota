# CC

> **macOS only.** There is no Windows or Linux build, and the source does not compile on them:
> frontmost-app detection binds to AppKit, and Claude sign-in is read from the macOS Keychain.
> Releases ship a single universal (Intel + Apple Silicon) macOS app.

CC is a local-first macOS menu bar utility for checking Codex and Claude quota from their existing local sign-in state.

This project is an MIT-licensed adaptation of [change-42-yhmm/quota-float](https://github.com/change-42-yhmm/quota-float). The upstream license, copyright notice, and attribution are retained.

Created and maintained by [Robin0725](https://github.com/Robin0725) (Robin). See [AUTHORS.md](AUTHORS.md) for attribution details.

## CC 0.3.12 highlights

- Shows Codex and Claude together as two compact horizontal quota capsules in the macOS menu bar.
- Enlarges each menu bar capsule to `40 × 17 pt` and scales the centered percentage with it, while staying inside the menu bar's safe visual height.
- Keeps the quota percentage at the exact visual center of each capsule, with five small bottom-edge status dots that never change the number's size or placement.
- Shows the 5-hour reset countdown through those dots: one lit dot equals one remaining started hour, so the last partial hour still shows one dot.
- Clicking the menu bar capsules opens the full CC menu.
- Uses the 5-hour quota whenever that window exists; only a missing 5-hour window falls back to weekly quota and receives a `W` marker.
- Preserves a real 0% 5-hour value instead of incorrectly falling back.
- Keeps the floating window optional and disabled by default.
- Uses a `100 × 100` transparent compact window with one dominant percentage; clicking keeps that trigger in place and opens a `320 × 320` Codex + Claude detail panel directly below it.
- Follows the frontmost macOS app: Codex shows Codex quota; every other app shows Claude quota. Clicking CC itself keeps the previous provider to avoid flicker.
- Uses tiny `CX / CL` and `5H / W` markers so the single number is never ambiguous.
- Keeps Codex cool blue and Claude warm orange, with restrained static gradients and no material animation.
- Expands only on click, never on hover, and separates a short click from window dragging with a movement threshold.
- Honors `prefers-reduced-motion` by removing the remaining idle transition.
- Keeps the last good value briefly as stale data and never invents quota when authentication or response formats fail.

## Menu bar

CC puts each exact percentage inside its provider-colored capsule. Codex is always the cool-blue capsule on the left; Claude is always the warm-orange capsule on the right. There are no provider initials or logos in the status item:

```text
[ 74% ] [ 94% ]
```

The capsule keeps the menu bar quiet; the menu and tooltip retain the provider name, window type, reset time, and stale-data detail:

```text
Codex · week 42% · 07/20 18:00 reset
```

Time dots appear only for the 5-hour window. When CC has to fall back to weekly quota, it omits the dots instead of pretending that five dots can represent a week; the exact weekly reset remains available in the menu and tooltip.

Open the menu to inspect reset times, refresh immediately, show or hide the floating window, toggle always-on-top, unlock mouse passthrough, switch language, control launch at login, or quit CC.

## How it works

CC reads the existing Codex Desktop and Claude Code login state on the same Mac, then calls each provider's quota service. It does not estimate quota from token counts, redeem reset credits, or modify account settings.

Codex authentication is read from `CODEX_HOME/auth.json` or `~/.codex/auth.json`. Claude Code authentication uses `CLAUDE_CODE_OAUTH_TOKEN` only when the user explicitly set it; otherwise the app reads the macOS Keychain item used by Claude Code, with a local Claude credentials-file fallback. Credentials are used in memory and are not copied into CC preferences.

Browser preview uses mock data. Real quota reading requires the Tauri desktop app and an existing Codex Desktop and/or Claude Code sign-in.

The provider quota endpoints may change. When an authentication method or response shape is no longer recognized, CC shows stale or unavailable state rather than fabricating a number.

## Privacy boundary

- Reads local sign-in state only to request quota.
- Sends each access token only to that provider's quota endpoint.
- Stores only widget preferences in the CC app config directory.
- Does not store tokens, account IDs, prompts, chat history, raw quota responses, or local auth paths.
- Includes no telemetry, analytics, crash reporting, or third-party tracking.
- Does not redeem reset credits or modify account settings.

See [PRIVACY.md](PRIVACY.md) and [SECURITY.md](SECURITY.md).

## Development

Requirements:

- Node.js 20.19+ or 22.12+
- Rust stable
- Tauri 2 system dependencies

```bash
npm install
npm run test
npm run build
npm run tauri dev
```

The visual preview is available in a browser with `npm run dev`; append `?designer=1` to open the CC design workbench. Use `?designer&mode=compare` for the old-versus-current comparison.

## Build

```bash
npm run tauri build
```

The transparent macOS WebView uses Tauri's `macOSPrivateApi`. Public builds should be distributed directly or through GitHub Releases rather than the Mac App Store.

Do not upload local credentials, `.codex`, `.claude`, `.env*`, personal screenshots, `node_modules`, `dist`, `src-tauri/target`, or local installers.

## License

MIT. See [LICENSE](LICENSE).

CC is an independent project and is not affiliated with or endorsed by OpenAI or Anthropic. Codex, OpenAI, Claude, and Anthropic are trademarks of their respective owners.
