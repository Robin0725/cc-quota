# CC 0.3.10 visual system

## Menu bar

- Two horizontal capsules, each `40 × 17 pt` at display scale.
- Codex is fixed on the left in cool blue; Claude is fixed on the right in warm orange.
- No provider logo or initials in the status item; position and color carry provider identity, while the menu and tooltip provide text labels.
- Percentage text remains centered regardless of value or time state.
- Five small dots sit at the bottom edge without shifting the percentage. For a 5-hour window, one lit dot equals one remaining started hour. Weekly fallback omits the dots.

## Floating window

- Compact native window: `100 × 100`; visible rounded trigger: `80 × 80` with a `10 px` transparent drag margin.
- Expanded native window: `320 × 430`, containing the anchored trigger, a `10 px` gap, and a `320 × 320` detail panel.
- Open below by default and above near the lower screen edge. Open toward the side with room so the trigger does not jump at the left or right edge.
- Compact mode shows one dominant percentage plus small `CX / CL` and `5H / W` identifiers.
- Detail mode shows both providers, current window, reset time, weekly quota, and stale state.

## Interaction and motion

- Short click expands; pointer movement beyond the drag threshold moves the native window.
- Detail mode has an explicit collapse button and supports Escape.
- No continuous material animation. `prefers-reduced-motion` removes the remaining idle opacity transition.
