import { memo, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { clampPercent, formatResetTime, preferredWindow } from "../lib/format";
import { copy, normalizeLanguage } from "../lib/i18n";
import type { Language, ProviderSnapshot } from "../types";

interface Props {
  snapshot: ProviderSnapshot | null;
  onDrag: () => void | Promise<void>;
  onHover: (hovered: boolean) => void;
  onToggleExpanded: () => void;
  expanded?: boolean;
  language?: Language;
}

interface DetailsProps {
  snapshots: ProviderSnapshot[];
  onDrag: () => void | Promise<void>;
  onToggleExpanded: () => void;
  language?: Language;
}

function providerAbbreviation(snapshot: ProviderSnapshot): string {
  return snapshot.provider === "codex" ? "CX" : "CL";
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

export const QuotaOrb = memo(function QuotaOrb({ snapshot, onDrag, onHover, onToggleExpanded, expanded = false, language = "zh-CN" }: Props) {
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<number | null>(null);
  const activeLanguage = normalizeLanguage(language);
  const interactions = useClickOrDrag(onToggleExpanded, onDrag);
  const selected = snapshot ? preferredWindow(snapshot) : null;
  const percent = selected ? clampPercent(selected.value.remainingPercent) : null;
  const available = snapshot !== null && selected !== null && percent !== null;

  useEffect(() => {
    idleTimer.current = window.setTimeout(() => setIdle(true), 2200);
    return () => {
      if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    };
  }, [snapshot?.provider]);

  const handleMouseEnter = () => {
    if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    setIdle(false);
    onHover(true);
  };

  const ariaLabel = available
    ? `${snapshot.displayName} ${copy[activeLanguage].availableLabel(percent, selected.kind)}，${activeLanguage === "en" ? "click for details" : "点击展开详情"}`
    : `${copy[activeLanguage].unavailableStatus}，${activeLanguage === "en" ? "click for details" : "点击展开详情"}`;

  return (
    <main
      className={`quota-orb${snapshot ? ` quota-orb--${snapshot.provider}` : " quota-orb--empty"}${idle ? " quota-orb--idle" : ""}`}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={() => onHover(false)}
      {...interactions}
      role="button"
      tabIndex={0}
      aria-label={ariaLabel}
      aria-expanded={expanded}
      aria-controls="quota-details-panel"
      data-cc-focus-target={expanded ? undefined : "true"}
    >
      {available ? (
        <section className="orb-metric" role="progressbar" aria-label={ariaLabel} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
          <span className="orb-source">{providerAbbreviation(snapshot)}<i aria-hidden="true" />{selected.kind === "weekly" ? "W" : "5H"}</span>
          <span className="orb-value">{percent}<small>%</small></span>
        </section>
      ) : (
        <section className="orb-empty" aria-live="polite"><span>CC</span><strong>—</strong></section>
      )}
    </main>
  );
});

export const QuotaDetails = memo(function QuotaDetails({ snapshots, onDrag, onToggleExpanded, language = "zh-CN" }: DetailsProps) {
  const activeLanguage = normalizeLanguage(language);
  const dragInteractions = useDrag(onDrag);
  const labels = activeLanguage === "en"
    ? { title: "Quota details", collapse: "Collapse details", drag: "Drag panel to move", short: "5-hour", weekly: "Weekly", noWeekly: "No weekly window", loading: "Reading quota" }
    : { title: "额度详情", collapse: "收起详情", drag: "拖动面板可移动", short: "5 小时", weekly: "周额度", noWeekly: "未返回周额度", loading: "正在读取额度" };

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
      <header className="details-header">
        <div><span>CC</span><strong>{labels.title}</strong></div>
        <div className="details-actions">
          <small>{labels.drag}</small>
          <button type="button" onClick={onToggleExpanded} aria-label={labels.collapse} data-cc-focus-target="true">
            <span aria-hidden="true">⌃</span>
          </button>
        </div>
      </header>
      <div className={`details-providers${snapshots.length === 1 ? " details-providers--single" : ""}`}>
        {snapshots.length > 0 ? snapshots.slice(0, 2).map((snapshot) => {
          const selected = preferredWindow(snapshot);
          const percent = selected ? clampPercent(selected.value.remainingPercent) : null;
          const weeklyPercent = snapshot.weeklyWindow ? clampPercent(snapshot.weeklyWindow.remainingPercent) : null;
          const isStale = snapshot.status === "stale";
          return (
            <section className={`detail-provider detail-provider--${snapshot.provider}`} key={snapshot.provider}>
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
                  <div className="detail-progress" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
                    <i style={{ width: `${percent}%` }} />
                  </div>
                  <div className="detail-meta">
                    <span>{formatResetTime(selected.value.resetsAt, new Date(), activeLanguage)}</span>
                    <span>{weeklyPercent === null ? labels.noWeekly : `${labels.weekly} ${weeklyPercent}%`}</span>
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
