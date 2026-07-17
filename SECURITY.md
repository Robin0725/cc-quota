# Security

## Supported Use

CC is a local macOS utility that reads Codex and Claude quota using the user's existing Codex Desktop and Claude Code login state.

## Reporting Issues

Please do not open public issues containing tokens, account IDs, raw backend responses, screenshots with personal data, or local file paths. Redact sensitive information before sharing logs or bug reports.

## Security Boundaries

- The app does not persist Codex or Claude credentials.
- The app does not log request headers or raw quota responses.
- The app caps auth file reads at 256 KB and quota responses at 1 MB.
- The app does not follow redirects for quota HTTP requests.
- The app does not redeem reset credits or change account settings.
- Claude Code Keychain access is read-only and may be denied by macOS; denial is shown as a signed-out/unavailable state.

## Release Notes For Maintainers

Before publishing a release, verify:

- Source archives do not include local installers, build outputs, `.codex`, `.claude`, QA screenshots, or environment files.
- macOS bundles are built by CI or a clean machine.
- Unsigned builds are clearly labeled as unsigned, and public signed releases use maintainer-controlled certificates.
- The upstream MIT license and attribution remain included in adapted releases.
