# CC Quota 0.3.13

CC is a local-first macOS menu bar quota monitor for Codex and Claude.

## Highlights

- Two provider-colored `40 × 17 pt` menu bar capsules with centered exact percentages.
- Five bottom-edge dots for the 5-hour window, where one lit dot equals one remaining started hour.
- Optional `100 × 100` transparent floating trigger that opens a `320 × 320` two-provider detail panel below or above it without moving the trigger.
- 5-hour priority, explicit weekly fallback, correct 0% semantics, and last-good stale-data retention.

## Download

- macOS Universal unsigned: `cc-macos-universal-unsigned.zip`

Unsigned builds may require right-clicking the app and choosing Open, or allowing it in System Settings → Privacy & Security.

## Verification

- Frontend and Rust tests passed.
- TypeScript/Vite production build and Rust clippy passed.
- Version and sensitive-content checks passed.
- Final macOS smoke test passed.
- Independent read-only review passed.

Created and maintained by [Robin0725](https://github.com/Robin0725). This adaptation retains the upstream MIT license and attribution.
