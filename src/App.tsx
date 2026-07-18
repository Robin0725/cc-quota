import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { QuotaDetails, QuotaOrb, type ProviderDescriptorMap } from "./components/QuotaCard";
import { fetchProviderDescriptors, fetchSnapshots, getFrontmostProvider, getPreferences, listenDesktopEvents, setWidgetExpanded, startDragging, type WidgetPlacement } from "./lib/bridge";
import { displayableSnapshots, needsFastRefresh } from "./lib/format";
import { copy, normalizeLanguage } from "./lib/i18n";
import { mergeSnapshots } from "./lib/snapshots";
import type { ProviderDescriptorDto, ProviderId, ProviderSnapshot, WidgetPreferences } from "./types";

const DEFAULT_PREFS: WidgetPreferences = { locked: false, alwaysOnTop: true, widgetVisible: false, pinnedProvider: null, autoRotateSeconds: 12, language: "zh-CN" };

/**
 * Registry order is display order. Providers the registry has not described yet (a snapshot that
 * arrives before the descriptors do) sort to the end rather than jumping to the front.
 */
function orderByDescriptors(items: ProviderSnapshot[], descriptors: ProviderDescriptorDto[]): ProviderSnapshot[] {
  const rank = (provider: ProviderId) => {
    const index = descriptors.findIndex((item) => item.id === provider);
    return index === -1 ? descriptors.length : index;
  };
  return [...items].sort((a, b) => rank(a.provider) - rank(b.provider));
}

export default function App() {
  const [snapshots, setSnapshots] = useState<ProviderSnapshot[]>([]);
  const [descriptors, setDescriptors] = useState<ProviderDescriptorDto[]>([]);
  const [preferences, setPreferences] = useState(DEFAULT_PREFS);
  const [frontmostProvider, setFrontmostProvider] = useState<ProviderId | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [expandedPlacement, setExpandedPlacement] = useState<WidgetPlacement>({ vertical: "below", horizontal: "right" });
  const expansionBusy = useRef(false);
  const failures = useRef(0);
  const language = normalizeLanguage(preferences.language);
  // `refresh` is stable (empty deps), so it reads the active copy through a ref instead of
  // hardcoding English into the snapshot message.
  const copyRef = useRef(copy[language]);
  copyRef.current = copy[language];

  // Five call sites (mount, interval, focus/visibility, hover, expand) can overlap. Only the most
  // recent request may write state, otherwise a slow earlier response overwrites fresher data and
  // concurrent responses each bump `failures`, corrupting the backoff interval.
  const latestRequest = useRef(0);

  const refresh = useCallback(async (force = false) => {
    const request = ++latestRequest.current;
    try {
      const values = await fetchSnapshots(force);
      if (request !== latestRequest.current) return;
      failures.current = values.some((item) => item.status !== "ok") ? failures.current + 1 : 0;
      setSnapshots((current) => mergeSnapshots(current, values));
    } catch {
      if (request !== latestRequest.current) return;
      failures.current += 1;
      setSnapshots((current) => current.map((item) => ({ ...item, status: "stale", message: copyRef.current.errorUnavailable })));
    }
  }, []);

  useEffect(() => {
    void refresh(true);
    void getPreferences()
      .then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }))
      .catch(() => undefined);
    // Descriptors are static for the process lifetime, so one fetch is enough.
    void fetchProviderDescriptors().then(setDescriptors).catch(() => undefined);
  }, [refresh]);

  useEffect(() => {
    let cancelled = false;
    const syncFrontmostProvider = async () => {
      try {
        const provider = await getFrontmostProvider();
        if (!cancelled && provider) setFrontmostProvider(provider);
      } catch {
        // Keep the last meaningful provider when macOS focus detection is unavailable.
      }
    };
    void syncFrontmostProvider();
    const id = window.setInterval(() => void syncFrontmostProvider(), 750);
    return () => { cancelled = true; window.clearInterval(id); };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let cleanup: () => void = () => {};
    void listenDesktopEvents({
      onPreferences: (value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }),
      onSnapshots: (value) => setSnapshots((current) => mergeSnapshots(current, value)),
    }).then((value) => { if (cancelled) value(); else cleanup = value; }).catch(() => undefined);
    return () => { cancelled = true; cleanup(); };
  }, []);

  const refreshMs = useMemo(() => {
    const backoff = failures.current === 0 ? 5 * 60_000 : Math.min(30 * 60_000, 30_000 * 2 ** (failures.current - 1));
    if (failures.current === 0 && snapshots.some((item) => item.status === "ok" && needsFastRefresh(item))) return 60_000;
    return backoff;
  }, [snapshots]);

  useEffect(() => {
    const id = window.setInterval(() => void refresh(), refreshMs);
    return () => window.clearInterval(id);
  }, [refresh, refreshMs]);

  useEffect(() => {
    const refreshWhenActive = () => { if (document.visibilityState === "visible") void refresh(); };
    window.addEventListener("focus", refreshWhenActive);
    document.addEventListener("visibilitychange", refreshWhenActive);
    return () => {
      window.removeEventListener("focus", refreshWhenActive);
      document.removeEventListener("visibilitychange", refreshWhenActive);
    };
  }, [refresh]);

  const available = useMemo(() => orderByDescriptors(displayableSnapshots(snapshots), descriptors), [snapshots, descriptors]);

  const current = available.find((item) => item.provider === frontmostProvider) ?? available[0] ?? null;

  const orderedSnapshots = useMemo(() => orderByDescriptors(snapshots, descriptors), [snapshots, descriptors]);

  const descriptorMap = useMemo<ProviderDescriptorMap>(
    () => Object.fromEntries(descriptors.map((item) => [item.id, item])),
    [descriptors],
  );

  const toggleExpanded = useCallback(() => {
    if (expansionBusy.current) return;
    const next = !expanded;
    expansionBusy.current = true;
    if (next) void refresh();
    void setWidgetExpanded(next)
      .then((placement) => {
        setExpandedPlacement(placement);
        setExpanded(next);
      })
      .finally(() => { expansionBusy.current = false; });
  }, [expanded, refresh]);

  useEffect(() => {
    if (preferences.widgetVisible || !expanded) return;
    setExpanded(false);
    void setWidgetExpanded(false);
  }, [expanded, preferences.widgetVisible]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-cc-focus-target='true']")?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [expanded]);

  if (expanded) {
    return (
      <div className={`expanded-widget expanded-widget--${expandedPlacement.vertical} expanded-widget--opens-${expandedPlacement.horizontal}`}>
        <div className="expanded-widget-trigger">
          <QuotaOrb
            snapshot={current}
            language={language}
            descriptors={descriptorMap}
            expanded
            onDrag={() => startDragging()}
            onHover={(value) => { if (value) void refresh(); }}
            onToggleExpanded={toggleExpanded}
          />
        </div>
        <div className="expanded-widget-panel">
          <QuotaDetails
            snapshots={orderedSnapshots}
            language={language}
            descriptors={descriptorMap}
            onDrag={() => startDragging()}
            onToggleExpanded={toggleExpanded}
          />
        </div>
      </div>
    );
  }

  return (
    <QuotaOrb
      snapshot={current}
      language={language}
      descriptors={descriptorMap}
      expanded={false}
      onDrag={() => startDragging()}
      onHover={(value) => { if (value) void refresh(); }}
      onToggleExpanded={toggleExpanded}
    />
  );
}
