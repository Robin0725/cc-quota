/**
 * A provider's stable id. Deliberately `string`, not a literal union: the registry lives in Rust
 * and the front end must never enumerate ids (`=== "codex"`), or adding a provider means editing
 * every switch that ever grew one.
 */
export type ProviderId = string;

/**
 * Presentation for one provider, defined once in the Rust `ProviderDescriptor` and pushed down.
 * Keeping abbreviation and accent colour on this side of the wire is what stops the tray bitmap
 * and the CSS from drifting apart.
 */
export interface ProviderDescriptorDto {
  id: string;
  displayName: string;
  abbreviation: string;
  accentHex: string;
}
export type SnapshotStatus = "ok" | "stale" | "loading" | "unavailable" | "signed_out";
export type Language = "zh-CN" | "en";

export interface UsageWindow {
  remainingPercent: number;
  resetsAt: string | null;
  windowSeconds: number;
}

/** A quota bucket scoped to one model, labelled by whatever name the API reports. */
export interface ScopedWindow {
  label: string;
  remainingPercent: number;
  resetsAt: string | null;
}

export interface ProviderSnapshot {
  provider: ProviderId;
  displayName: string;
  plan: string | null;
  shortWindow: UsageWindow | null;
  weeklyWindow: UsageWindow | null;
  scopedWindows?: ScopedWindow[];
  resetCredits: number | null;
  resetCreditExpiresAt?: string[];
  updatedAt: string;
  status: SnapshotStatus;
  message: string | null;
}

export interface WidgetPreferences {
  locked: boolean;
  alwaysOnTop: boolean;
  widgetVisible: boolean;
  pinnedProvider: ProviderId | null;
  autoRotateSeconds: number;
  language: Language;
}
