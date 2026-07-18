import type { ProviderSnapshot } from "../types";
import { preferredWindow } from "./format";

export function mergeSnapshots(current: ProviderSnapshot[], incoming: ProviderSnapshot[]): ProviderSnapshot[] {
  const merged: ProviderSnapshot[] = incoming.map((next) => {
    if (next.status === "ok") return next;
    if (next.status === "signed_out") return next;
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
