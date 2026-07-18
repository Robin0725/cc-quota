# Release guide

The current source version is CC 0.3.12. The three manifests and a release tag must match exactly.

## Validation

CI enforces every command below except `npm run build`, on both `push` and `pull_request`, so a
failing check blocks the merge rather than waiting to be noticed here. Run them locally anyway to
avoid a round trip through CI.

```bash
npm ci
npm run check:version
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

For a tagged build, `npm run check:version -- v0.3.12` must also pass.

## Distribution boundary

The workflow builds `cc-macos-universal-unsigned.zip`. It creates a draft GitHub Release so a maintainer can inspect the artifact before publishing it. The app is not signed with an Apple Developer ID and is not notarized, so Gatekeeper may warn or block it.

The transparent WebView uses Tauri `macOSPrivateApi`; distribute through GitHub Releases or directly, not through the Mac App Store.

Never include credentials, local config, `node_modules`, `dist`, `src-tauri/target`, personal screenshots, or local app bundles in a source release.
