import { useState } from "react";
import originalOrbReference from "../../docs/images/quota-orb.png";
import type { ProviderSnapshot } from "../types";
import { QuotaDetails, QuotaOrb } from "./QuotaCard";

const now = Date.now();

const codex: ProviderSnapshot = {
  provider: "codex", displayName: "CODEX", plan: "PRO",
  shortWindow: { remainingPercent: 74, resetsAt: new Date(now + 90 * 60_000).toISOString(), windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 42, resetsAt: new Date(now + 3 * 86_400_000).toISOString(), windowSeconds: 604_800 },
  resetCredits: 1, resetCreditExpiresAt: [], updatedAt: new Date(now).toISOString(), status: "ok", message: null,
};

const claude: ProviderSnapshot = {
  provider: "claude", displayName: "CLAUDE", plan: "MAX",
  shortWindow: { remainingPercent: 91, resetsAt: new Date(now + 2 * 60 * 60_000).toISOString(), windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 86, resetsAt: new Date(now + 4 * 86_400_000).toISOString(), windowSeconds: 604_800 },
  resetCredits: null, resetCreditExpiresAt: [], updatedAt: new Date(now).toISOString(), status: "ok", message: null,
};

const weeklyCodex: ProviderSnapshot = { ...codex, shortWindow: null, weeklyWindow: { ...codex.weeklyWindow!, remainingPercent: 42 } };

// The expanded panel had no preview mode, which is how a misplaced per-model row reached a build.
// Fable shares the account weekly's reset instant, so it is given the same resetsAt here.
const claudeWithScoped: ProviderSnapshot = {
  ...claude,
  scopedWindows: [{ label: "Fable", remainingPercent: 75, resetsAt: claude.weeklyWindow!.resetsAt }],
};

type PreviewMode = "codex" | "claude" | "weekly" | "empty" | "details" | "compare";
const modes: Array<{ value: PreviewMode; label: string }> = [
  { value: "codex", label: "Codex · 5H" },
  { value: "claude", label: "Claude · 5H" },
  { value: "weekly", label: "Codex · 周额度" },
  { value: "empty", label: "暂无额度" },
  { value: "details", label: "展开面板" },
  { value: "compare", label: "旧版对照" },
];

function initialMode(): PreviewMode {
  const mode = new URLSearchParams(window.location.search).get("mode") as PreviewMode | null;
  return modes.some((item) => item.value === mode) ? mode! : "codex";
}

export function DesignPlayground() {
  const [mode, setMode] = useState<PreviewMode>(() => initialMode());
  const screenshotMode = new URLSearchParams(window.location.search).has("shot");
  const snapshot = mode === "codex" ? codex : mode === "claude" ? claude : mode === "weekly" ? weeklyCodex : null;
  const preview = mode === "details"
    ? <div className="design-panel-frame"><QuotaDetails snapshots={[codex, claudeWithScoped]} language="zh-CN" onDrag={() => undefined} onToggleExpanded={() => undefined} /></div>
    : <div className="design-orb-frame"><QuotaOrb snapshot={snapshot} language="zh-CN" onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} /></div>;

  if (mode === "compare") {
    return (
      <div className="comparison-stage">
        <figure><figcaption>旧版原图</figcaption><img src={originalOrbReference} alt="旧版单百分比悬浮窗" /></figure>
        <figure><figcaption>当前实现</figcaption><div className="comparison-canvas"><div className="comparison-orb"><QuotaOrb snapshot={codex} language="zh-CN" onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} /></div></div></figure>
      </div>
    );
  }

  if (screenshotMode) return <div className="screenshot-stage">{preview}</div>;

  return (
    <div className="design-workbench">
      <section className="design-stage">
        <nav className="design-preview-switch" aria-label="CC visual preview">
          {modes.map((item) => <button type="button" key={item.value} className={mode === item.value ? "is-active" : ""} onClick={() => setMode(item.value)}>{item.label}</button>)}
        </nav>
        {preview}
      </section>
      <aside className="design-controls">
        <p className="design-kicker">CC / SIMPLE QUOTA</p>
        <h1>一个窗，一个数</h1>
        <p>固定 100 × 100 透明窗口；只显示当前服务的一项额度。前台是 Codex 时显示 Codex，其余应用显示 Claude。</p>
        <p>Codex 使用冷蓝雾面，Claude 使用暖橙雾面。无展开、无液体、无莫比乌斯、无复杂材质动画。</p>
      </aside>
    </div>
  );
}
