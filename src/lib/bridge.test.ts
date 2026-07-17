import { describe, expect, it } from "vitest";
import { calculateCompactWidgetAnchor, calculateExpandedWidgetLayout } from "./bridge";

describe("floating widget placement", () => {
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
