# Current design QA

The current design uses one quiet capsule per signed-in provider and an optional floating detail view.

- Percentages stay centered inside enlarged menu bar capsules.
- Time dots occupy the bottom edge and never move or resize the number.
- Left click switches between the unchanged 5-hour view and weekly reset countdowns; right click opens the full menu.
- Weekly quota at 20% or below gains 2% alternating fade graduations and a solid white 10% marker.
- The compact floating trigger stays anchored when details open below, above, left, or right.
- Color and fixed position distinguish providers in the menu bar; text labels remain available in the menu and tooltip.
- The floating trigger retains small text identifiers because it can switch providers based on the frontmost app.
- The detail view exposes an explicit collapse control and keeps quota meters semantically readable.

Files under `docs/images` are public product and design references. Validate current behavior in the installed macOS app before treating a static image as runtime evidence.
