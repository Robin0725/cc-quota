import { memo, useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { clampPercent, formatResetTime, hoursUntilReset, preferredWindow, quotaTier, RESET_DOT_COUNT } from "../lib/format";
import { copy, normalizeLanguage } from "../lib/i18n";
import type { CSSProperties } from "react";
import type { Language, ProviderDescriptorDto, ProviderSnapshot } from "../types";

/** Descriptors keyed by provider id, as handed down from the Rust registry. */
export type ProviderDescriptorMap = Record<string, ProviderDescriptorDto>;

interface Props {
  snapshot: ProviderSnapshot | null;
  onDrag: () => void | Promise<void>;
  onHover: (hovered: boolean) => void;
  onToggleExpanded: () => void;
  expanded?: boolean;
  language?: Language;
  descriptors?: ProviderDescriptorMap;
}

interface DetailsProps {
  snapshots: ProviderSnapshot[];
  onDrag: () => void | Promise<void>;
  onToggleExpanded: () => void;
  language?: Language;
  descriptors?: ProviderDescriptorMap;
}

/**
 * The descriptor owns the label; the fallback only covers the window between mount and the
 * registry's first reply, and must never guess a *specific* provider's mark.
 */
function providerAbbreviation(snapshot: ProviderSnapshot, descriptors?: ProviderDescriptorMap): string {
  return descriptors?.[snapshot.provider]?.abbreviation ?? snapshot.displayName.slice(0, 2).toUpperCase();
}

/**
 * Every provider-tinted surface reads `--provider-accent`; nothing inherits a colour it did not
 * declare. Leaving the variable unset falls back to neutral grey in CSS, never to another
 * provider's hue.
 */
function accentStyle(snapshot: ProviderSnapshot, descriptors?: ProviderDescriptorMap): CSSProperties | undefined {
  const accent = descriptors?.[snapshot.provider]?.accentHex;
  return accent ? ({ "--provider-accent": accent } as CSSProperties) : undefined;
}

function useClickOrDrag(onActivate: () => void, onDrag: () => void | Promise<void>) {
  const pointerStart = useRef<{ x: number; y: number } | null>(null);
  const dragging = useRef(false);

  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    pointerStart.current = { x: event.screenX, y: event.screenY };
    dragging.current = false;
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    if (!pointerStart.current || dragging.current || (event.buttons & 1) === 0) return;
    const distance = Math.hypot(event.screenX - pointerStart.current.x, event.screenY - pointerStart.current.y);
    if (distance < 5) return;
    dragging.current = true;
    void onDrag();
  };

  const onPointerUp = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || !pointerStart.current) return;
    const shouldActivate = !dragging.current;
    pointerStart.current = null;
    dragging.current = false;
    if (shouldActivate) onActivate();
  };

  const onPointerCancel = () => {
    pointerStart.current = null;
    dragging.current = false;
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onActivate();
  };

  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onKeyDown };
}

function useDrag(onDrag: () => void | Promise<void>) {
  const pointerStart = useRef<{ x: number; y: number } | null>(null);
  const dragging = useRef(false);

  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    pointerStart.current = { x: event.screenX, y: event.screenY };
    dragging.current = false;
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    if (!pointerStart.current || dragging.current || (event.buttons & 1) === 0) return;
    const distance = Math.hypot(event.screenX - pointerStart.current.x, event.screenY - pointerStart.current.y);
    if (distance < 5) return;
    dragging.current = true;
    void onDrag();
  };

  const reset = () => {
    pointerStart.current = null;
    dragging.current = false;
  };

  return { onPointerDown, onPointerMove, onPointerUp: reset, onPointerCancel: reset };
}

export const QuotaOrb = memo(function QuotaOrb({ snapshot, onDrag, onHover, onToggleExpanded, expanded = false, language = "zh-CN", descriptors }: Props) {
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<number | null>(null);
  const activeLanguage = normalizeLanguage(language);
  const interactions = useClickOrDrag(onToggleExpanded, onDrag);
  const selected = snapshot ? preferredWindow(snapshot) : null;
  const percent = selected ? clampPercent(selected.value.remainingPercent) : null;
  const available = snapshot !== null && selected !== null && percent !== null;
  // Only the 5-hour window gets a countdown, matching `snapshot_time_hours` in the tray renderer:
  // a weekly window would light every dot for days on end and tell the user nothing.
  const resetHours = selected?.kind === "short" ? hoursUntilReset(selected.value) : null;

  const startIdleTimer = useCallback(() => {
    if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    idleTimer.current = window.setTimeout(() => setIdle(true), 2200);
  }, []);

  useEffect(() => {
    startIdleTimer();
    return () => {
      if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    };
  }, [snapshot?.provider, startIdleTimer]);

  const handleMouseEnter = () => {
    if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    setIdle(false);
    onHover(true);
  };

  // Without restarting the timer here the orb stays fully opaque forever after the first hover,
  // because the effect above only re-runs when the provider changes.
  const handleMouseLeave = () => {
    startIdleTimer();
    onHover(false);
  };

  const t = copy[activeLanguage];
  const ariaLabel = available
    ? `${snapshot.displayName} ${t.availableLabel(percent, selected.kind)}${t.clauseSeparator}${t.expandDetails}`
    : `${t.unavailableStatus}${t.clauseSeparator}${t.expandDetails}`;
  const tier = quotaTier(percent);

  return (
    <main
      className={`quota-orb${snapshot ? "" : " quota-orb--empty"} quota-orb--tier-${tier}${idle ? " quota-orb--idle" : ""}`}
      style={snapshot ? accentStyle(snapshot, descriptors) : undefined}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      {...interactions}
      role="button"
      tabIndex={0}
      aria-label={ariaLabel}
      aria-expanded={expanded}
      aria-controls="quota-details-panel"
      // The panel has no collapse button, so the orb stays the focus target in both states: it is
      // what toggles the panel, and Escape is handled globally while expanded.
      data-cc-focus-target="true"
    >
      {available ? (
        <section className="orb-metric" role="progressbar" aria-label={ariaLabel} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
          <span className="orb-source">{providerAbbreviation(snapshot, descriptors)}<i aria-hidden="true" />{selected.kind === "weekly" ? "W" : "5H"}</span>
          <span className="orb-value">{percent}<small>%</small></span>
          {resetHours !== null ? (
            <span className="orb-dots" aria-hidden="true">
              {Array.from({ length: RESET_DOT_COUNT }, (_, index) => (
                <i key={index} className={index < resetHours ? "is-lit" : undefined} />
              ))}
            </span>
          ) : null}
        </section>
      ) : (
        <section className="orb-empty" aria-live="polite"><span>CC</span><strong>—</strong></section>
      )}
    </main>
  );
});

/**
 * The panel's ambient backdrop: one soft glow per provider actually on screen, walked along the
 * same diagonal the hand-tuned blue/orange pair used to sit on. Two providers land exactly where
 * the old hardcoded stops were; one centres the single glow; four spread evenly without crowding.
 *
 * Colours come from the descriptors rather than a fixed pair, so a third provider tints the
 * backdrop instead of leaving it advertising the two that shipped first.
 */
function backdropGlow(snapshots: ProviderSnapshot[], descriptors?: ProviderDescriptorMap): CSSProperties | undefined {
  const accents = snapshots.map((item) => descriptors?.[item.provider]?.accentHex).filter((hex): hex is string => Boolean(hex));
  if (accents.length === 0) return undefined;
  const layers = accents.map((accent, index) => {
    const t = accents.length === 1 ? 0.5 : index / (accents.length - 1);
    const x = 22 + 60 * t;
    const y = 25 + 53 * t;
    const alpha = 25 - 3 * t;
    const spread = 34 + 2 * t;
    return `radial-gradient(circle at ${x}% ${y}%, color-mix(in srgb, ${accent} ${alpha}%, transparent), transparent ${spread}%)`;
  });
  return { backgroundImage: layers.join(", ") };
}

export const QuotaDetails = memo(function QuotaDetails({ snapshots, onDrag, onToggleExpanded, language = "zh-CN", descriptors }: DetailsProps) {
  const activeLanguage = normalizeLanguage(language);
  const dragInteractions = useDrag(onDrag);
  const labels = activeLanguage === "en"
    ? { title: "Quota details", short: "5-hour", weekly: "Weekly", noWeekly: "No weekly window", loading: "Reading quota" }
    : { title: "额度详情", short: "5 小时", weekly: "周额度", noWeekly: "未返回周额度", loading: "正在读取额度" };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onToggleExpanded();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onToggleExpanded]);

  return (
    <main
      id="quota-details-panel"
      className="quota-details"
      {...dragInteractions}
      aria-label={labels.title}
    >
      <div className="details-glow" style={backdropGlow(snapshots, descriptors)} aria-hidden="true" />
      <header className="details-header">
        <div><span>CC</span><strong>{labels.title}</strong></div>
      </header>
      {/* Scrolling is decided by CSS overflow, not by counting providers here: how tall a card is
          depends on its content, so a count could never be the right trigger. */}
      <div className="details-providers">
        {snapshots.length > 0 ? snapshots.map((snapshot) => {
          const selected = preferredWindow(snapshot);
          const percent = selected ? clampPercent(selected.value.remainingPercent) : null;
          const weeklyPercent = snapshot.weeklyWindow ? clampPercent(snapshot.weeklyWindow.remainingPercent) : null;
          const isStale = snapshot.status === "stale";
          return (
            <section className="detail-provider" style={accentStyle(snapshot, descriptors)} key={snapshot.provider}>
              <div className="detail-provider-heading">
                <div><i aria-hidden="true" /><strong>{snapshot.displayName}</strong></div>
                <span>{snapshot.plan ?? copy[activeLanguage].accountFallback}{isStale ? " · STALE" : ""}</span>
              </div>
              {selected && percent !== null ? (
                <>
                  <div className="detail-primary">
                    <strong>{percent}<small>%</small></strong>
                    <span>{selected.kind === "short" ? labels.short : labels.weekly}</span>
                  </div>
                  <div className={`detail-progress detail-progress--tier-${quotaTier(percent)}`} role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
                    <i style={{ width: `${percent}%` }} />
                  </div>
                  {/* Meta and scoped buckets share a wrapper so the tight three-card layout can fold
                      them onto one line. They stay stacked at one and two providers. */}
                  <div className="detail-footer">
                    <div className="detail-meta">
                      <span>{formatResetTime(selected.value.resetsAt, new Date(), activeLanguage)}</span>
                      <span>{weeklyPercent === null ? labels.noWeekly : `${labels.weekly} ${weeklyPercent}%`}</span>
                    </div>
                    {(snapshot.scopedWindows ?? []).length > 0 ? (
                      <div className="detail-scoped">
                        {(snapshot.scopedWindows ?? []).map((scoped) => (
                          <span key={scoped.label}>
                            {scoped.label}{" "}
                            <span className={`detail-scoped-value detail-scoped-value--${quotaTier(clampPercent(scoped.remainingPercent))}`}>
                              {clampPercent(scoped.remainingPercent)}%
                            </span>
                          </span>
                        ))}
                      </div>
                    ) : null}
                  </div>
                </>
              ) : (
                <p className="detail-unavailable">{snapshot.message ?? copy[activeLanguage].unavailableStatus}</p>
              )}
            </section>
          );
        }) : <p className="details-loading">{labels.loading}</p>}
      </div>
    </main>
  );
});
