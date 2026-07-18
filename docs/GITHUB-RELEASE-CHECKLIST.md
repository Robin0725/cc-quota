# GitHub release checklist

## Source release

- The repository is public under the intended owner.
- `LICENSE`, `AUTHORS.md`, the upstream link, and Robin0725 attribution are present.
- `npm run check:version`, frontend tests/build, Rust format/tests/clippy, and the sensitive-content scan pass.
- The staged file list contains no credentials, generated dependencies, build output, private screenshots, or internal review logs.
- The app is manually smoke-tested on macOS: menu, both quota capsules, floating-window show/hide, click-to-expand/collapse, drag, screen-edge placement, and stale-data behavior.

## Tagged build

```bash
git tag v0.3.11
git push origin v0.3.11
```

The tag check rejects a tag that does not match all three manifests. The release workflow creates a draft with `cc-macos-universal-unsigned.zip`; inspect the artifact before publishing it.

For broad public distribution, add Apple Developer ID signing and notarization first.
