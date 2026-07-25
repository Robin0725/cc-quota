import { describe, expect, it } from "vitest";
import {
  FOCUS_REFRESH_COOLDOWN_MS,
  FOCUS_REFRESH_DELAY_MS,
  focusRefreshCooldownDelay,
  focusRefreshDelays,
  focusedKimiRecoveryDelay,
  KIMI_FOCUSED_RECOVERY_DELAYS_MS,
} from "./active-provider";

describe("active provider refresh policy", () => {
  it("does not duplicate the initial all-provider fetch", () => {
    expect(focusRefreshDelays(null, "codex", false)).toEqual([]);
  });

  it("does not refetch while focus stays on the same provider", () => {
    expect(focusRefreshDelays("kimicode", "kimicode", true)).toEqual([]);
  });

  it("debounces an ordinary provider change into one forced read", () => {
    expect(focusRefreshDelays("codex", "claude", true)).toEqual([FOCUS_REFRESH_DELAY_MS]);
  });

  it("uses one immediate read when focus moves to Kimi", () => {
    expect(focusRefreshDelays("claude", "kimicode", true)).toEqual([FOCUS_REFRESH_DELAY_MS]);
  });

  it("refreshes when the first completed observation was null", () => {
    expect(focusRefreshDelays(null, "kimicode", true)).toEqual([FOCUS_REFRESH_DELAY_MS]);
  });

  it("coalesces rapid focus churn behind a one-second refresh cooldown", () => {
    expect(focusRefreshCooldownDelay(1_400, 1_000)).toBe(FOCUS_REFRESH_COOLDOWN_MS - 400);
    expect(focusRefreshCooldownDelay(2_000, 1_000)).toBe(0);
    expect(focusRefreshCooldownDelay(2_500, 1_000)).toBe(0);
  });

  it("recovers when Kimi renews credentials without another provider transition", () => {
    expect(focusedKimiRecoveryDelay("kimicode", null, 0)).toBe(KIMI_FOCUSED_RECOVERY_DELAYS_MS[0]);
    expect(focusedKimiRecoveryDelay("kimicode", "unavailable", 1)).toBe(KIMI_FOCUSED_RECOVERY_DELAYS_MS[1]);
    expect(focusedKimiRecoveryDelay("kimicode", "stale", 3)).toBe(KIMI_FOCUSED_RECOVERY_DELAYS_MS[3]);
    expect(focusedKimiRecoveryDelay("kimicode", "stale", 4)).toBeNull();
    expect(focusedKimiRecoveryDelay("kimicode", "stale", 99)).toBeNull();
  });

  it("stops focused recovery after success, sign-out, or leaving Kimi", () => {
    expect(focusedKimiRecoveryDelay("kimicode", "ok", 0)).toBeNull();
    expect(focusedKimiRecoveryDelay("kimicode", "signed_out", 0)).toBeNull();
    expect(focusedKimiRecoveryDelay("claude", "unavailable", 0)).toBeNull();
    expect(focusedKimiRecoveryDelay(null, "unavailable", 0)).toBeNull();
  });
});
