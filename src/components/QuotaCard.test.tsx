// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
// Read off disk so the assertions below inspect the real stylesheet: jsdom never applies it,
// so `getComputedStyle` would report nothing useful, and a `?raw` import is stubbed empty here.
import { readFileSync } from "node:fs";
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

// A third provider the components have never been told about, standing in for anything the Rust
// registry may add later.
const kimicode: ProviderSnapshot = {
  ...codex,
  provider: "kimicode",
  displayName: "KIMI CODE",
  plan: "INTERMEDIATE",
  shortWindow: { ...codex.shortWindow!, remainingPercent: 37 },
  weeklyWindow: { ...codex.weeklyWindow!, remainingPercent: 44 },
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

  // Parameterised on purpose: the panel must render whatever the registry hands it, so the count
  // is data rather than a constant. The three-provider row is what a `slice(0, 2)` would silently
  // eat, which is exactly the bug this locks out.
  it.each([
    { label: "two providers", snapshots: [codex, claude], names: ["CODEX", "CLAUDE"], percents: ["84", "61"] },
    { label: "three providers", snapshots: [codex, claude, kimicode], names: ["CODEX", "CLAUDE", "KIMI CODE"], percents: ["84", "61", "37"] },
    { label: "one provider", snapshots: [codex], names: ["CODEX"], percents: ["84"] },
  ])("renders every provider the registry reports ($label)", ({ snapshots, names, percents }) => {
    render(<QuotaDetails snapshots={snapshots} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    for (const name of names) expect(screen.getByText(name)).toBeTruthy();
    expect(screen.getAllByRole("meter").map((item) => item.getAttribute("aria-valuenow"))).toEqual(percents);
    // The panel has no chrome of its own: collapsing is the orb's job, or Escape.
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("renders every card when far more providers arrive than the panel can show at once", () => {
    const many = [codex, claude, kimicode, { ...kimicode, provider: "d", displayName: "D" }, { ...kimicode, provider: "e", displayName: "E" }];
    const { container } = render(<QuotaDetails snapshots={many} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(container.querySelectorAll(".detail-provider")).toHaveLength(5);
  });

  // Regression: Claude's scoped `Fable` row is the last element in the card, so it was the first
  // thing a too-short row cut off. It rendered in the DOM the whole time — which is exactly why a
  // DOM-presence assertion alone cannot catch this class of bug.
  it("keeps a scoped bucket rendered under a three-provider layout", () => {
    const withFable: ProviderSnapshot = {
      ...claude,
      scopedWindows: [{ label: "Fable", remainingPercent: 25, resetsAt: "2026-07-21T03:00:00Z" }],
    };
    render(<QuotaDetails snapshots={[codex, withFable, kimicode]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(screen.getByText("Fable")).toBeTruthy();
    expect(screen.getByText("25%")).toBeTruthy();
  });

  it("takes the orb's provider mark and accent colour from the descriptor", () => {
    const descriptors = { kimicode: { id: "kimicode", displayName: "Kimi Code", abbreviation: "KM", accentHex: "#7c5cd6" } };
    const { container } = render(
      <QuotaOrb snapshot={kimicode} descriptors={descriptors} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} />,
    );

    expect(container.querySelector(".orb-source")?.textContent).toBe("KM5H");
    expect(container.querySelector<HTMLElement>(".quota-orb")?.style.getPropertyValue("--provider-accent")).toBe("#7c5cd6");
  });

  // The backdrop used to be a hardcoded blue+orange pair, which would have gone on advertising two
  // providers no matter how many were actually on screen.
  it.each([1, 2, 3])("tints the panel backdrop from the providers actually shown (%i)", (count) => {
    const all = [codex, claude, kimicode];
    const descriptors = {
      codex: { id: "codex", displayName: "Codex", abbreviation: "CX", accentHex: "#2f6fed" },
      claude: { id: "claude", displayName: "Claude", abbreviation: "CL", accentHex: "#b85a3a" },
      kimicode: { id: "kimicode", displayName: "Kimi Code", abbreviation: "KM", accentHex: "#7c5cd6" },
    };
    const { container } = render(
      <QuotaDetails snapshots={all.slice(0, count)} descriptors={descriptors} onDrag={() => undefined} onToggleExpanded={() => undefined} />,
    );

    const glow = container.querySelector<HTMLElement>(".details-glow")!.style.backgroundImage;
    expect(glow.match(/radial-gradient/g)).toHaveLength(count);
    for (const accent of all.slice(0, count).map((item) => descriptors[item.provider as keyof typeof descriptors].accentHex)) {
      expect(glow).toContain(accent);
    }
  });

  it("leaves the backdrop neutral before any descriptor has arrived", () => {
    const { container } = render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={() => undefined} />);

    expect(container.querySelector<HTMLElement>(".details-glow")?.style.backgroundImage).toBe("");
  });

  /*
   * These assert on the stylesheet text rather than on rendered geometry, because jsdom performs
   * no layout: every height it reports is 0, so it can neither see a clipped row nor prove one is
   * impossible. The limitation is real — this cannot verify the panel *looks* right, only that the
   * specific style combination which made silent clipping possible has not come back.
   *
   * That combination was: a grid row allowed to shrink below its content (`minmax(0, 1fr)`) sitting
   * under a card that hides its overflow. Either alone is harmless; together they delete content
   * with no visual trace. A real layout check belongs in a browser-driven screenshot test.
   */
  describe("detail panel cannot silently clip card content", () => {
    const stylesheet = // Path is relative to the project root, where `npm test` runs.
      readFileSync("src/styles.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    const declarationsFor = (selector: string) =>
      [...stylesheet.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
        .filter((rule) => rule[1].split(",").some((part: string) => part.trim() === selector))
        .map((rule) => rule[2])
        .join(" ");

    it("never lets a provider row shrink below the height of the card inside it", () => {
      const grid = declarationsFor(".details-providers");

      expect(grid).toMatch(/grid-auto-rows:\s*minmax\(\s*min-content/);
      expect(grid).not.toMatch(/grid-auto-rows:\s*minmax\(\s*0/);
    });

    it("does not hide overflow on the card, so anything too tall stays visible", () => {
      expect(declarationsFor(".detail-provider")).not.toMatch(/overflow:\s*hidden/);
    });

    it("scrolls the list on overflow instead of at a hardcoded provider count", () => {
      expect(declarationsFor(".details-providers")).toMatch(/overflow-y:\s*auto/);
      // A count-keyed scroll class would be blind to how tall the cards actually are.
      expect(stylesheet).not.toContain("details-providers--scroll");
    });
  });

  it("collapses the detail view with Escape", () => {
    const onToggleExpanded = vi.fn();
    render(<QuotaDetails snapshots={[codex, claude]} onDrag={() => undefined} onToggleExpanded={onToggleExpanded} />);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onToggleExpanded).toHaveBeenCalledTimes(1);
  });
});
