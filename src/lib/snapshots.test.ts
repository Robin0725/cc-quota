import { describe, expect, it } from "vitest";
import type { ProviderSnapshot } from "../types";
import { mergeSnapshots } from "./snapshots";

const success: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 74, resetsAt: "2026-07-07T02:00:00Z", windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 42, resetsAt: "2026-07-10T00:00:00Z", windowSeconds: 604_800 },
  resetCredits: 1,
  updatedAt: "2026-07-07T00:00:00Z",
  status: "ok",
  message: null,
};

describe("snapshot failure handling", () => {
  it("retains the last successful values during a transient failure", () => {
    const failure: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "unavailable", message: "Network unavailable", updatedAt: "2026-07-07T01:00:00Z" };
    expect(mergeSnapshots([success], [failure])[0]).toEqual({ ...success, status: "stale", message: "Network unavailable" });
  });

  it("shows a failure when no successful snapshot exists", () => {
    const signedOut: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "signed_out", message: "Please sign in" };
    expect(mergeSnapshots([], [signedOut])[0].status).toBe("signed_out");
  });

  it("does not hide an expired login behind stale quota data", () => {
    const signedOut: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "signed_out", message: "Please sign in" };
    expect(mergeSnapshots([success], [signedOut])[0].status).toBe("signed_out");
  });

  it("keeps a provider missing from a partial response instead of dropping it from the UI", () => {
    const claude: ProviderSnapshot = { ...success, provider: "claude", displayName: "CLAUDE" };
    const merged = mergeSnapshots([success, claude], [success]);

    expect(merged.map((item) => item.provider).sort()).toEqual(["claude", "codex"]);
    expect(merged.find((item) => item.provider === "claude")?.status).toBe("stale");
    expect(merged.find((item) => item.provider === "codex")?.status).toBe("ok");
  });

  it("does not resurrect a provider that never had usable quota data", () => {
    const empty: ProviderSnapshot = { ...success, provider: "claude", shortWindow: null, weeklyWindow: null, status: "signed_out" };
    expect(mergeSnapshots([success, empty], [success]).map((item) => item.provider)).toEqual(["codex"]);
  });

  it("replaces stale data after recovery", () => {
    expect(mergeSnapshots([{ ...success, status: "stale" }], [{ ...success, shortWindow: { ...success.shortWindow!, remainingPercent: 88 } }])[0].shortWindow?.remainingPercent).toBe(88);
  });
});
