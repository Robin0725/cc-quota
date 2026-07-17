import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { QuotaDetails, QuotaOrb } from "./components/QuotaCard";
import { fetchSnapshots, getFrontmostProvider, getPreferences, listenDesktopEvents, setWidgetExpanded, startDragging, type WidgetPlacement } from "./lib/bridge";
import { displayableSnapshots, needsFastRefresh } from "./lib/format";
import { normalizeLanguage } from "./lib/i18n";
import { mergeSnapshots } from "./lib/snapshots";
import type { ProviderId, ProviderSnapshot, WidgetPreferences } from "./types";

const DEFAULT_PREFS: WidgetPreferences = { locked: false, alwaysOnTop: true, widgetVisible: false, pinnedProvider: null, autoRotateSeconds: 12, language: "zh-CN" };
const PROVIDER_ORDER = ["codex", "claude"];

export default function App() {
  const [snapshots, setSnapshots] = useState<ProviderSnapshot[]>([]);
  const [preferences, setPreferences] = useState(DEFAULT_PREFS);
  const [frontmostProvider, setFrontmostProvider] = useState<ProviderId | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [expandedPlacement, setExpandedPlacement] = useState<WidgetPlacement>({ vertical: "below", horizontal: "left" });
  const expansionBusy = useRef(false);
  const failures = useRef(0);
  const language = normalizeLanguage(preferences.language);

  const refresh = useCallback(async (force = false) => {
    try {
      const values = await fetchSnapshots(force);
      failures.current = values.some((item) => item.status !== "ok") ? failures.current + 1 : 0;
      setSnapshots((current) => mergeSnapshots(current, values));
    } catch {
      failures.current += 1;
      setSnapshots((current) => current.map((item) => ({ ...item, status: "stale", message: "Refresh failed. Please try again later." })));
    }
  }, []);

  useEffect(() => {
    void refresh(true);
    void getPreferences()
      .then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }))
      .catch(() => undefined);
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

  const available = useMemo(() => displayableSnapshots(snapshots).sort((a, b) => PROVIDER_ORDER.indexOf(a.provider) - PROVIDER_ORDER.indexOf(b.provider)), [snapshots]);

  const current = available.find((item) => item.provider === frontmostProvider) ?? available[0] ?? null;

  const orderedSnapshots = useMemo(
    () => [...snapshots].sort((a, b) => PROVIDER_ORDER.indexOf(a.provider) - PROVIDER_ORDER.indexOf(b.provider)),
    [snapshots],
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
      expanded={false}
      onDrag={() => startDragging()}
      onHover={(value) => { if (value) void refresh(); }}
      onToggleExpanded={toggleExpanded}
    />
  );
}
