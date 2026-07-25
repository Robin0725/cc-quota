import { describe, expect, it } from "vitest";
import {
  FOCUS_REFRESH_COOLDOWN_MS,
  FOCUS_REFRESH_DELAY_MS,
  focusRefreshCooldownDelay,
  focusRefreshDelays,
  KIMI_TOKEN_RENEWAL_RETRY_MS,
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

  it("retries Kimi once after its CLI has had time to renew the token", () => {
    expect(focusRefreshDelays("claude", "kimicode", true)).toEqual([
      FOCUS_REFRESH_DELAY_MS,
      KIMI_TOKEN_RENEWAL_RETRY_MS,
    ]);
  });

  it("refreshes when the first completed observation was null", () => {
    expect(focusRefreshDelays(null, "kimicode", true)).toEqual([
      FOCUS_REFRESH_DELAY_MS,
      KIMI_TOKEN_RENEWAL_RETRY_MS,
    ]);
  });

  it("coalesces rapid focus churn behind a one-second refresh cooldown", () => {
    expect(focusRefreshCooldownDelay(1_400, 1_000)).toBe(FOCUS_REFRESH_COOLDOWN_MS - 400);
    expect(focusRefreshCooldownDelay(2_000, 1_000)).toBe(0);
    expect(focusRefreshCooldownDelay(2_500, 1_000)).toBe(0);
  });
});
