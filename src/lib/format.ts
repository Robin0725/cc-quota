import { copy, normalizeLanguage } from "./i18n";
import type { Language, ProviderSnapshot, UsageWindow } from "../types";

export type PreferredWindow = {
  kind: "short" | "weekly";
  value: UsageWindow;
};

export function preferredWindow(snapshot: ProviderSnapshot): PreferredWindow | null {
  if (snapshot.shortWindow) return { kind: "short", value: snapshot.shortWindow };
  if (snapshot.weeklyWindow) return { kind: "weekly", value: snapshot.weeklyWindow };
  return null;
}

export function isSnapshotDisplayable(snapshot: ProviderSnapshot, now = new Date()): boolean {
  if (!preferredWindow(snapshot)) return false;
  if (snapshot.status === "ok") return true;
  if (snapshot.status !== "stale") return false;
  const updatedAt = new Date(snapshot.updatedAt).getTime();
  return Number.isFinite(updatedAt) && now.getTime() - updatedAt <= 30 * 60_000;
}

export function displayableSnapshots(snapshots: ProviderSnapshot[], now = new Date()): ProviderSnapshot[] {
  return snapshots.filter((snapshot) => isSnapshotDisplayable(snapshot, now));
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, Math.round(value)));
}

export function quotaTier(percent: number | null): "unknown" | "healthy" | "caution" | "critical" {
  if (percent === null) return "unknown";
  if (percent >= 50) return "healthy";
  if (percent >= 10) return "caution";
  return "critical";
}

export function formatResetTime(value: string | null, now = new Date(), language: Language = "zh-CN"): string {
  const t = copy[normalizeLanguage(language)];
  if (!value) return t.resetTimeUnknown;
  const target = new Date(value);
  if (Number.isNaN(target.getTime())) return t.resetTimeUnknown;
  const delta = target.getTime() - now.getTime();
  if (delta <= 0) return t.resetUpdating;
  const minutes = Math.ceil(delta / 60_000);
  if (minutes < 60) return t.resetInMinutes(minutes);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours < 24) return t.resetInHours(hours, rest);
  const days = Math.floor(hours / 24);
  return t.resetInDays(days, hours % 24);
}

/** Dots lit on the orb, mirroring `time_remaining_hours` in the tray renderer. */
export const RESET_DOT_COUNT = 5;

/**
 * Whole hours left in a window, rounded up and capped at {@link RESET_DOT_COUNT}.
 *
 * Returns 0 once the window has elapsed so the countdown reads empty rather than disappearing,
 * and null when there is nothing to count down to.
 */
export function hoursUntilReset(window: UsageWindow | null | undefined, now = new Date()): number | null {
  if (!window?.resetsAt) return null;
  const target = new Date(window.resetsAt).getTime();
  if (Number.isNaN(target)) return null;
  const delta = target - now.getTime();
  if (delta <= 0) return 0;
  return Math.min(RESET_DOT_COUNT, Math.max(1, Math.ceil(delta / 3_600_000)));
}

export function needsFastRefresh(snapshot: ProviderSnapshot, now = new Date()): boolean {
  const reset = preferredWindow(snapshot)?.value.resetsAt;
  if (!reset) return false;
  const remaining = new Date(reset).getTime() - now.getTime();
  return remaining > -5 * 60_000 && remaining <= 15 * 60_000;
}

export function formatResetDate(value: string | null, language: Language = "zh-CN"): string {
  const t = copy[normalizeLanguage(language)];
  if (!value) return t.dateUnknown;
  const isoDate = /^(\d{4})-(\d{2})-(\d{2})/.exec(value);
  if (isoDate) {
    return `${Number(isoDate[2])}/${Number(isoDate[3])}`;
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return t.dateUnknown;
  return new Intl.DateTimeFormat(language === "en" ? "en-US" : "zh-CN", { month: "numeric", day: "numeric" }).format(date);
}

export function formatDateTime(value: string, language: Language): string {
  const t = copy[normalizeLanguage(language)];
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return t.creditExpiresUnknown;
  return new Intl.DateTimeFormat(language === "en" ? "en-US" : "zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(date);
}
