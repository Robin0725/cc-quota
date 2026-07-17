# Test matrix

| Area | Required behavior | Automated evidence | Manual evidence |
| --- | --- | --- | --- |
| Window selection | 5-hour wins; 0% stays 0%; weekly only when short window is absent | Frontend and Rust parser tests | Real provider accounts |
| Failure handling | Transient failure keeps last-good stale values; signed-out clears them | Frontend and Rust merge tests | Disconnect/sign-out smoke test |
| Menu bar | Two colored capsules, centered percentages, distinct 0/unknown, hour dots | Rust pixel tests | macOS status item inspection |
| Floating placement | Trigger remains anchored; opens below/above and left/right based on work area | Bridge layout tests | Test all screen edges |
| Floating interaction | Click expands, drag does not expand, collapse button and Escape work | Component tests | macOS click/drag/lock test |
| Accessibility | Meter semantics remain exposed; focus moves to the active control; reduced motion is honored | Component/CSS inspection | Keyboard and VoiceOver smoke test |
| Privacy | No secrets in source or staged files; credentials never persist | Static scan and code review | Inspect app config and logs |
| Build | Manifests/tag agree; frontend, Rust, clippy, and production build pass | CI and local commands | Launch built app |

## Release gate

Do not publish while a P0/P1 review finding remains open, the final staged file set is unreviewed, or any required command fails.
