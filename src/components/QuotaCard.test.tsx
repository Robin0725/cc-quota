// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderSnapshot } from "../types";
import { QuotaDetails, QuotaOrb } from "./QuotaCard";

if (!window.PointerEvent) {
  window.PointerEvent = MouseEvent as typeof PointerEvent;
}

const codex: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 84, resetsAt: "2026-07-17T20:00:00Z", windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 72, resetsAt: "2026-07-23T20:00:00Z", windowSeconds: 604_800 },
  resetCredits: 1,
  resetCreditExpiresAt: [],
  updatedAt: "2026-07-17T08:00:00Z",
  status: "ok",
  message: null,
};

const claude: ProviderSnapshot = {
  ...codex,
  provider: "claude",
  displayName: "CLAUDE",
  plan: "MAX",
  shortWindow: { ...codex.shortWindow!, remainingPercent: 61 },
  weeklyWindow: { ...codex.weeklyWindow!, remainingPercent: 53 },
};

afterEach(cleanup);

describe("floating widget interactions", () => {
  it("treats a short press as expand instead of drag", () => {
    const onToggleExpanded = vi.fn();
    const onDrag = vi.fn();
    render(<QuotaOrb snapshot={codex} onDrag={onDrag} onHover={() => undefined} onToggleExpanded={onToggleExpanded} />);

    const widget = screen.getByRole("button", { name: /点击展开详情/ });
    fireEvent.pointerDown(widget, { button: 0, buttons: 1, screenX: 20, screenY: 20 });
    fireEvent.pointerUp(widget, { button: 0, screenX: 20, screenY: 20 });

    expect(onToggleExpanded).toHaveBeenCalledTimes(1);
    expect(onDrag).not.toHaveBeenCalled();
  });

  it("starts dragging after movement and does not expand", () => {
    const onToggleExpanded = vi.fn();
    const onDrag = vi.fn();
    render(<QuotaOrb snapshot={codex} onDrag={onDrag} onHover={() => undefined} onToggleExpanded={onToggleExpanded} />);

    const widget = screen.getByRole("button", { name: /点击展开详情/ });
    fireEvent.pointerDown(widget, { button: 0, buttons: 1, screenX: 20, screenY: 20 });
    fireEvent.pointerMove(widget, { buttons: 1, screenX: 28, screenY: 20 });
    fireEvent.pointerUp(widget, { button: 0, screenX: 28, screenY: 20 });

    expect(onDrag).toHaveBeenCalledTimes(1);
    expect(onToggleExpanded).not.toHaveBeenCalled();
  });

  it("dims again after every hover, not just the first one", () => {
    vi.useFakeTimers();
    try {
      render(<QuotaOrb snapshot={codex} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} />);
      const widget = screen.getByRole("button", { name: /点击展开详情/ });

      act(() => { vi.advanceTimersByTime(2200); });
      expect(widget.className).toContain("quota-orb--idle");

      fireEvent.mouseEnter(widget);
      expect(widget.className).not.toContain("quota-orb--idle");

      fireEvent.mouseLeave(widget);
      act(() => { vi.advanceTimersByTime(2200); });
      expect(widget.className).toContain("quota-orb--idle");
    } finally {
      vi.useRealTimers();
    }
  });

  it("flags a nearly exhausted quota without relying on the provider colour", () => {
    const critical: ProviderSnapshot = { ...codex, shortWindow: { ...codex.shortWindow!, remainingPercent: 4 } };
    const { container } = render(<QuotaOrb snapshot={critical} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} />);

    expect(container.querySelector(".quota-orb--tier-critical")).toBeTruthy();
  });

  it("lights one dot per remaining hour of the 5-hour window", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-17T17:30:00Z")); // 2.5h before the 20:00Z reset
    try {
      const { container } = render(<QuotaOrb snapshot={codex} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} />);

      expect(container.querySelectorAll(".orb-dots i")).toHaveLength(5);
      expect(container.querySelectorAll(".orb-dots i.is-lit")).toHaveLength(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("omits the countdown when only a weekly window is available", () => {
    const weeklyOnly: ProviderSnapshot = { ...codex, shortWindow: null };
    const { container } = render(<QuotaOrb snapshot={weeklyOnly} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} />);

    expect(container.querySelector(".orb-dots")).toBeNull();
  });

  it("shows a per-model bucket under the provider it belongs to", () => {
    const withFable: ProviderSnapshot = {
      ...claude,
      scopedWindows: [{ label: "Fable", remainingPercent: 75, resetsAt: "2026-07-21T03:00:00Z" }],
    };
    render(<QuotaDetails snapshots={[codex, withFable]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(screen.getByText("Fable")).toBeTruthy();
    expect(screen.getByText("75%")).toBeTruthy();
  });

  it("shows nothing extra when the provider reports no per-model bucket", () => {
    const { container } = render(<QuotaDetails snapshots={[codex]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(container.querySelector(".detail-scoped")).toBeNull();
  });

  it("renders both providers in the expanded detail view", () => {
    render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(screen.getByText("CODEX")).toBeTruthy();
    expect(screen.getByText("CLAUDE")).toBeTruthy();
    expect(screen.getAllByRole("meter").map((item) => item.getAttribute("aria-valuenow"))).toEqual(["84", "61"]);
    // The panel has no chrome of its own: collapsing is the orb's job, or Escape.
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("collapses the detail view with Escape", () => {
    const onToggleExpanded = vi.fn();
    render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={onToggleExpanded} />);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onToggleExpanded).toHaveBeenCalledTimes(1);
  });
});
