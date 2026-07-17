// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

  it("renders both providers in the expanded detail view", () => {
    render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(screen.getByText("CODEX")).toBeTruthy();
    expect(screen.getByText("CLAUDE")).toBeTruthy();
    expect(screen.getAllByRole("meter").map((item) => item.getAttribute("aria-valuenow"))).toEqual(["84", "61"]);
    expect(screen.getByRole("button", { name: "收起详情" })).toBeTruthy();
  });

  it("collapses the detail view with Escape", () => {
    const onToggleExpanded = vi.fn();
    render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={onToggleExpanded} />);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onToggleExpanded).toHaveBeenCalledTimes(1);
  });
});
