# CC 0.3.10 design QA

The current design uses a quiet dual-capsule status item and an optional floating detail view.

- Percentages stay centered inside enlarged menu bar capsules.
- Time dots occupy the bottom edge and never move or resize the number.
- The compact floating trigger stays anchored when details open below, above, left, or right.
- Color and fixed position distinguish providers in the menu bar; text labels remain available in the menu and tooltip.
- The floating trigger retains small text identifiers because it can switch providers based on the frontmost app.
- The detail view exposes an explicit collapse control and keeps quota meters semantically readable.

Files under `docs/images` are local design references used by the development-only comparison view; they are not claims about the current production layout.
