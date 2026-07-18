import type { ProviderSnapshot } from "../types";
import { preferredWindow } from "./format";

export function mergeSnapshots(current: ProviderSnapshot[], incoming: ProviderSnapshot[]): ProviderSnapshot[] {
  const merged: ProviderSnapshot[] = incoming.map((next) => {
    // Every failure keeps the last good reading, whatever its status. Statuses are not a reliable
    // signal of permanence: an expired short-lived token reports `signed_out` while the provider's
    // own CLI is about to renew it, and letting that erase the numbers made the card vanish
    // instead of dimming. `isSnapshotDisplayable` bounds how long a stale reading may be shown.
    if (next.status === "ok") return next;
    const previous = current.find((item) => item.provider === next.provider && preferredWindow(item));
    return previous
      ? { ...previous, status: "stale", message: next.message, updatedAt: previous.updatedAt }
      : next;
  });

  // A partial response must not make a provider vanish from the UI; carry it over as stale so it
  // degrades the same way a reported failure would.
  const reported = new Set(merged.map((item) => item.provider));
  const carried: ProviderSnapshot[] = current
    .filter((item) => !reported.has(item.provider) && preferredWindow(item))
    .map((item) => ({ ...item, status: "stale" }));

  return [...merged, ...carried];
}
