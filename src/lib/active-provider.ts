import type { ProviderId } from "../types";

export const FOCUS_REFRESH_DELAY_MS = 150;
export const FOCUS_REFRESH_COOLDOWN_MS = 1_000;
export const KIMI_TOKEN_RENEWAL_RETRY_MS = 2_000;

/** Keeps rapid focus churn from turning into one all-provider refresh every observation tick. */
export function focusRefreshCooldownDelay(now: number, lastRefresh: number): number {
  return Math.max(0, FOCUS_REFRESH_COOLDOWN_MS - (now - lastRefresh));
}

/**
 * The initial focus observation races the initial all-provider fetch, so it needs no second read.
 * Every real provider transition does: Kimi may have renewed its short-lived token as its CLI
 * came to the foreground, while the cached snapshot still says unavailable.
 */
export function focusRefreshDelays(
  previous: ProviderId | null,
  next: ProviderId | null,
  initialized: boolean,
): number[] {
  if (!initialized || next === null || previous === next) return [];
  return next === "kimicode"
    ? [FOCUS_REFRESH_DELAY_MS, KIMI_TOKEN_RENEWAL_RETRY_MS]
    : [FOCUS_REFRESH_DELAY_MS];
}
