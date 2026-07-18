# Privacy

CC is designed to be local-first and minimal.

## What It Reads

- The app reads the local Codex Desktop login file from `CODEX_HOME/auth.json` or the user's `.codex/auth.json`.
- The app sends the existing Codex access token only to the ChatGPT quota endpoints needed to read Codex usage.
- The app may read the account identifier from the login file or token payload only to set the request header expected by the quota service.
- If the user explicitly set `CLAUDE_CODE_OAUTH_TOKEN`, the app uses that process environment value. Otherwise, on macOS it reads the Claude Code credential item from Keychain service `Claude Code-credentials`; if unavailable, it may read the Claude Code credentials file under `CLAUDE_CONFIG_DIR` or `~/.claude`.
- The app sends the existing Claude Code OAuth token only to Anthropic's Claude usage endpoint.
- The app reads the local Kimi Code OAuth credentials from `KIMI_CODE_HOME/credentials` or the user's `.kimi-code/credentials`, and sends that token only to the Kimi Code usage endpoint. The app reads the access token only: it never uses the refresh token, never asks for a new token, and never writes to the credentials file. An expired token is simply reported as a failed reading, leaving the Kimi Code CLI in sole control of its own login.
- To decide which assistant is currently in use, the app subscribes to file system change events for each CLI's session directory. It records only the time of the most recent reported change for each provider. **It does not open, read, parse, or index any session file**, and it does not retain their names or paths. Session files contain user conversations; CC never looks inside them.

## What It Stores

CC stores only widget preferences in its own application config directory:

- locked state
- always-on-top state
- floating-widget visibility
- display language

In memory only, and never written to disk, CC also holds the time of the most recent session-directory change per provider, which is how it decides whose quota to show.

Legacy `pinnedProvider` and `autoRotateSeconds` fields may remain in migrated 0.2 preferences for compatibility, but CC does not use them for provider switching.

It does not copy or persist Codex, Claude, or Kimi Code tokens, account IDs, raw quota responses, user prompts, chat history, or local file paths.

## What It Sends

The app only calls these quota-related HTTPS endpoints from the local desktop process:

- `https://chatgpt.com/backend-api/wham/usage`
- `https://chatgpt.com/backend-api/wham/rate-limit-reset-credits`
- `https://api.anthropic.com/api/oauth/usage`
- `https://api.kimi.com/coding/v1/usages`

No telemetry, analytics, crash reporting, or third-party tracking is included.

## Logging

Logs are intentionally generic. They must not include tokens, account IDs, raw backend responses, request headers, local auth paths, or personal file paths.

## Accuracy Boundary

CC displays quota windows returned by the Codex and Claude quota services. It does not estimate quota from local token usage and does not fabricate values when a response shape is unknown.
