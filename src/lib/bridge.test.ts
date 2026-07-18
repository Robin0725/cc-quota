import { describe, expect, it } from "vitest";
import { calculateCompactWidgetAnchor, calculateExpandedWidgetLayout, resolveCompactAnchor } from "./bridge";

describe("floating widget placement", () => {
  it("opens down and to the right when the orb has room on both sides", () => {
    // Mid-screen, so neither direction is forced: this is the case that pins the default.
    const layout = calculateExpandedWidgetLayout(
      { x: 400, y: 60, width: 100, height: 100 },
      { x: 0, y: 24, width: 1200, height: 876 },
    );

    expect(layout.position).toEqual({ x: 400, y: 60 });
    expect(layout.placement).toEqual({ vertical: "below", horizontal: "right" });
  });

  it("keeps the trigger anchored and opens the detail panel below it", () => {
    const layout = calculateExpandedWidgetLayout(
      { x: 900, y: 60, width: 100, height: 100 },
      { x: 0, y: 24, width: 1200, height: 876 },
    );

    expect(layout).toEqual({
      position: { x: 680, y: 60 },
      size: { width: 320, height: 430 },
      placement: { vertical: "below", horizontal: "left" },
    });
  });

  it("flips the detail panel above only when there is not enough room below", () => {
    const layout = calculateExpandedWidgetLayout(
      { x: 900, y: 750, width: 100, height: 100 },
      { x: 0, y: 24, width: 1200, height: 876 },
    );

    expect(layout.position).toEqual({ x: 680, y: 420 });
    expect(layout.placement).toEqual({ vertical: "above", horizontal: "left" });
  });

  it("opens to the right without moving a trigger near the left edge", () => {
    const layout = calculateExpandedWidgetLayout(
      { x: 8, y: 60, width: 100, height: 100 },
      { x: 0, y: 24, width: 1200, height: 876 },
    );

    expect(layout.position.x).toBe(8);
    expect(layout.placement.horizontal).toBe("right");
  });

  it("returns the orb to its pre-expand spot when the panel had to be pushed on-screen", () => {
    // A work area too short for the 430pt panel: expanding shifts the window up, and the
    // geometric anchor alone would leave the orb stranded at the shifted position.
    const work = { x: 0, y: 24, width: 1200, height: 400 };
    const orb = { x: 400, y: 100, width: 100, height: 100 };
    const layout = calculateExpandedWidgetLayout(orb, work);
    expect(layout.position.y).toBe(32);

    const restore = { expandedOrigin: layout.position, compactAnchor: { x: orb.x, y: orb.y } };
    expect(resolveCompactAnchor(layout.position, layout.placement, restore)).toEqual({ x: 400, y: 100 });
    expect(calculateCompactWidgetAnchor(layout.position, layout.placement)).toEqual({ x: 400, y: 32 });
  });

  it("ignores the remembered spot once the expanded window has been dragged", () => {
    const restore = { expandedOrigin: { x: 400, y: 32 }, compactAnchor: { x: 400, y: 100 } };
    const dragged = { x: 700, y: 200 };

    expect(resolveCompactAnchor(dragged, { vertical: "below", horizontal: "right" }, restore)).toEqual(dragged);
  });

  it("falls back to geometry when nothing was remembered", () => {
    expect(resolveCompactAnchor({ x: 700, y: 150 }, { vertical: "below", horizontal: "left" }, null))
      .toEqual({ x: 920, y: 150 });
  });

  it("restores the compact anchor from the expanded window's current position", () => {
    expect(calculateCompactWidgetAnchor(
      { x: 700, y: 150 },
      { vertical: "below", horizontal: "left" },
    )).toEqual({ x: 920, y: 150 });
    expect(calculateCompactWidgetAnchor(
      { x: 50, y: 200 },
      { vertical: "above", horizontal: "right" },
    )).toEqual({ x: 50, y: 530 });
  });
});
