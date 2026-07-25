import type { ProviderId, SnapshotStatus } from "../types";

export const FOCUS_REFRESH_DELAY_MS = 150;
export const FOCUS_REFRESH_COOLDOWN_MS = 1_000;
export const KIMI_FOCUSED_RECOVERY_DELAYS_MS = [2_000, 5_000, 10_000, 30_000] as const;

/** Keeps rapid focus churn from turning into one all-provider refresh every observation tick. */
export function focusRefreshCooldownDelay(now: number, lastRefresh: number): number {
  return Math.max(0, FOCUS_REFRESH_COOLDOWN_MS - (now - lastRefresh));
}

/**
 * The initial focus observation races the initial all-provider fetch, so it needs no second read.
 * Every real provider transition does. Kimi recovery after this first read is handled by the
 * focused snapshot policy below, so the two schedulers never issue duplicate two-second reads.
 */
export function focusRefreshDelays(
  previous: ProviderId | null,
  next: ProviderId | null,
  initialized: boolean,
): number[] {
  if (!initialized || next === null || previous === next) return [];
  return [FOCUS_REFRESH_DELAY_MS];
}

/**
 * Kimi may renew its token without changing the focused provider (for example, launching a new
 * CLI from an already focused Kimi terminal). Keep retrying with a capped backoff only while Kimi
 * is focused and its quota snapshot is recoverable. Attempts are finite because a prior good
 * reading deliberately turns every later failure — including a real sign-out — into `stale`.
 */
export function focusedKimiRecoveryDelay(
  provider: ProviderId | null,
  status: SnapshotStatus | null,
  attempt: number,
): number | null {
  if (
    provider !== "kimicode"
    || status === "ok"
    || status === "signed_out"
    || attempt < 0
    || attempt >= KIMI_FOCUSED_RECOVERY_DELAYS_MS.length
  ) return null;
  return KIMI_FOCUSED_RECOVERY_DELAYS_MS[attempt];
}
