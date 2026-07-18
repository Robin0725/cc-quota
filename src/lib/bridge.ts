import type { ProviderId, ProviderSnapshot, WidgetPreferences } from "../types";

const defaultPreferences: WidgetPreferences = { locked: false, alwaysOnTop: true, widgetVisible: false, pinnedProvider: null, autoRotateSeconds: 12, language: "zh-CN" };
const compactWidgetSize = { width: 100, height: 100 };
const expandedPanelSize = { width: 320, height: 320 };
const expandedPanelGap = 10;
const monitorInset = 8;

export type WidgetPlacement = {
  vertical: "below" | "above";
  horizontal: "left" | "right";
};

type LogicalPoint = { x: number; y: number };
type LogicalRect = LogicalPoint & { width: number; height: number };

let expandedWidgetPlacement: WidgetPlacement = { vertical: "below", horizontal: "left" };

export function calculateExpandedWidgetLayout(compact: LogicalRect, workArea?: LogicalRect): {
  position: LogicalPoint;
  size: { width: number; height: number };
  placement: WidgetPlacement;
} {
  const size = {
    width: expandedPanelSize.width,
    height: compactWidgetSize.height + expandedPanelGap + expandedPanelSize.height,
  };
  const opensLeftX = compact.x + compact.width - size.width;
  const opensRightX = compact.x;
  let x = opensLeftX;
  let y = compact.y;
  const placement: WidgetPlacement = { vertical: "below", horizontal: "left" };

  if (workArea) {
    const minX = workArea.x + monitorInset;
    const maxX = Math.max(minX, workArea.x + workArea.width - size.width - monitorInset);
    const opensLeftFits = opensLeftX >= minX && opensLeftX <= maxX;
    const opensRightFits = opensRightX >= minX && opensRightX <= maxX;
    if (!opensLeftFits && opensRightFits) {
      x = opensRightX;
      placement.horizontal = "right";
    } else {
      x = Math.min(maxX, Math.max(minX, opensLeftX));
    }

    const minY = workArea.y + monitorInset;
    const maxY = Math.max(minY, workArea.y + workArea.height - size.height - monitorInset);
    const fitsBelow = compact.y + size.height <= workArea.y + workArea.height - monitorInset;
    const aboveY = compact.y - expandedPanelSize.height - expandedPanelGap;
    const fitsAbove = aboveY >= minY;
    if (!fitsBelow && fitsAbove) {
      y = aboveY;
      placement.vertical = "above";
    } else {
      y = Math.min(maxY, Math.max(minY, y));
    }
  }

  return { position: { x, y }, size, placement };
}

export function calculateCompactWidgetAnchor(expanded: LogicalPoint, placement: WidgetPlacement): LogicalPoint {
  return {
    x: expanded.x + (placement.horizontal === "left" ? expandedPanelSize.width - compactWidgetSize.width : 0),
    y: expanded.y + (placement.vertical === "above" ? expandedPanelSize.height + expandedPanelGap : 0),
  };
}

const mockCodexSnapshot: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 74, resetsAt: new Date(Date.now() + 78 * 60_000).toISOString(), windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 42, resetsAt: new Date(Date.now() + 3.2 * 86_400_000).toISOString(), windowSeconds: 604_800 },
  resetCredits: 1,
  resetCreditExpiresAt: [new Date(Date.now() + 9 * 86_400_000).toISOString()],
  updatedAt: new Date().toISOString(),
  status: "ok",
  message: null,
};

const mockClaudeSnapshot: ProviderSnapshot = {
  provider: "claude",
  displayName: "CLAUDE",
  plan: "MAX",
  shortWindow: { remainingPercent: 94, resetsAt: new Date(Date.now() + 112 * 60_000).toISOString(), windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 86, resetsAt: new Date(Date.now() + 4.5 * 86_400_000).toISOString(), windowSeconds: 604_800 },
  resetCredits: null,
  resetCreditExpiresAt: [],
  updatedAt: new Date().toISOString(),
  status: "ok",
  message: null,
};

export const isTauri = () => "__TAURI_INTERNALS__" in window;

export async function fetchSnapshots(force = false): Promise<ProviderSnapshot[]> {
  if (!isTauri()) return [mockCodexSnapshot, mockClaudeSnapshot];
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ProviderSnapshot[]>(force ? "refresh_snapshots" : "get_snapshots");
}

export async function getPreferences(): Promise<WidgetPreferences> {
  if (!isTauri()) return defaultPreferences;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("get_preferences");
}

export async function updatePreferences(value: WidgetPreferences): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_preferences", { preferences: value });
}

export async function setClickThrough(locked: boolean): Promise<WidgetPreferences> {
  if (!isTauri()) return { ...defaultPreferences, locked };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("set_widget_locked", { locked });
}

export async function setAlwaysOnTop(alwaysOnTop: boolean): Promise<WidgetPreferences> {
  if (!isTauri()) return { ...defaultPreferences, alwaysOnTop };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("set_widget_always_on_top", { alwaysOnTop });
}

export async function setWidgetVisible(visible: boolean): Promise<WidgetPreferences> {
  if (!isTauri()) return { ...defaultPreferences, widgetVisible: visible };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("set_widget_visible", { visible });
}

export async function startDragging(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().startDragging();
}

export async function setWidgetExpanded(expanded: boolean): Promise<WidgetPlacement> {
  if (!isTauri()) return { vertical: "below", horizontal: "left" };
  const { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } = await import("@tauri-apps/api/window");
  const appWindow = getCurrentWindow();
  const [physicalPosition, monitor] = await Promise.all([
    appWindow.outerPosition(),
    currentMonitor(),
  ]);
  const scaleFactor = monitor?.scaleFactor ?? await appWindow.scaleFactor();
  const position = physicalPosition.toLogical(scaleFactor);

  if (expanded) {
    const workArea = monitor ? (() => {
      const workPosition = monitor.workArea.position.toLogical(scaleFactor);
      const workSize = monitor.workArea.size.toLogical(scaleFactor);
      return { x: workPosition.x, y: workPosition.y, width: workSize.width, height: workSize.height };
    })() : undefined;
    const layout = calculateExpandedWidgetLayout(
      { x: position.x, y: position.y, ...compactWidgetSize },
      workArea,
    );
    expandedWidgetPlacement = layout.placement;
    await appWindow.setPosition(new LogicalPosition(Math.round(layout.position.x), Math.round(layout.position.y)));
    await appWindow.setSize(new LogicalSize(layout.size.width, layout.size.height));
    return layout.placement;
  }

  let anchor = calculateCompactWidgetAnchor(position, expandedWidgetPlacement);
  if (monitor) {
    const workPosition = monitor.workArea.position.toLogical(scaleFactor);
    const workSize = monitor.workArea.size.toLogical(scaleFactor);
    anchor = {
      x: Math.min(
        workPosition.x + workSize.width - compactWidgetSize.width - monitorInset,
        Math.max(workPosition.x + monitorInset, anchor.x),
      ),
      y: Math.min(
        workPosition.y + workSize.height - compactWidgetSize.height - monitorInset,
        Math.max(workPosition.y + monitorInset, anchor.y),
      ),
    };
  }
  await appWindow.setSize(new LogicalSize(compactWidgetSize.width, compactWidgetSize.height));
  await appWindow.setPosition(new LogicalPosition(Math.round(anchor.x), Math.round(anchor.y)));
  expandedWidgetPlacement = { vertical: "below", horizontal: "left" };
  return expandedWidgetPlacement;
}

export async function getFrontmostProvider(): Promise<ProviderId | null> {
  if (!isTauri()) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ProviderId | null>("get_frontmost_provider");
}

export async function listenDesktopEvents(handlers: {
  onPreferences: (value: WidgetPreferences) => void;
  onSnapshots: (value: ProviderSnapshot[]) => void;
}): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  const { listen } = await import("@tauri-apps/api/event");
  const unlistenPreferences = await listen<WidgetPreferences>("preferences-changed", (event) => handlers.onPreferences(event.payload));
  let unlistenSnapshots: () => void;
  try {
    unlistenSnapshots = await listen<ProviderSnapshot[]>("snapshots-changed", (event) => handlers.onSnapshots(event.payload));
  } catch (error) {
    // Otherwise the first listener leaks: its handle is lost and the caller swallows the rejection.
    unlistenPreferences();
    throw error;
  }
  return () => { unlistenPreferences(); unlistenSnapshots(); };
}
