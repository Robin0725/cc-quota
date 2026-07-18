import { useState } from "react";
import originalOrbReference from "../../docs/images/quota-orb.png";
import { mockProviderDescriptors, mockProviderSnapshots } from "../lib/bridge";
import type { ProviderSnapshot } from "../types";
import { QuotaDetails, QuotaOrb, type ProviderDescriptorMap } from "./QuotaCard";

const descriptors = mockProviderDescriptors();
const descriptorMap: ProviderDescriptorMap = Object.fromEntries(descriptors.map((item) => [item.id, item]));
const [codex] = mockProviderSnapshots();

const weeklyCodex: ProviderSnapshot = { ...codex, shortWindow: null, weeklyWindow: { ...codex.weeklyWindow!, remainingPercent: 42 } };

// The panel preview shows every mock provider, Claude's scoped `Fable` bucket included, so the
// tallest layout the panel has to survive is the one on screen by default.
const panelSnapshots = mockProviderSnapshots();

// Modes are generated from the registry, so a new provider shows up in the playground switcher
// without anyone remembering to add a case here.
type PreviewMode = string;
const providerModes = descriptors.map((item) => ({ value: item.id, label: `${item.displayName} · 5H` }));
const modes: Array<{ value: PreviewMode; label: string }> = [
  ...providerModes,
  { value: "weekly", label: `${descriptors[0]?.displayName ?? ""} · 周额度` },
  { value: "empty", label: "暂无额度" },
  { value: "details", label: "展开面板" },
  { value: "compare", label: "旧版对照" },
];

const snapshotsByProvider = new Map(mockProviderSnapshots().map((item) => [item.provider, item]));

function initialMode(): PreviewMode {
  const mode = new URLSearchParams(window.location.search).get("mode");
  return modes.some((item) => item.value === mode) ? mode! : modes[0].value;
}

export function DesignPlayground() {
  const [mode, setMode] = useState<PreviewMode>(() => initialMode());
  const screenshotMode = new URLSearchParams(window.location.search).has("shot");
  const snapshot = mode === "weekly" ? weeklyCodex : snapshotsByProvider.get(mode) ?? null;
  const preview = mode === "details"
    ? <div className="design-panel-frame"><QuotaDetails snapshots={panelSnapshots} language="zh-CN" descriptors={descriptorMap} onDrag={() => undefined} onToggleExpanded={() => undefined} /></div>
    : <div className="design-orb-frame"><QuotaOrb snapshot={snapshot} language="zh-CN" descriptors={descriptorMap} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} /></div>;

  if (mode === "compare") {
    return (
      <div className="comparison-stage">
        <figure><figcaption>旧版原图</figcaption><img src={originalOrbReference} alt="旧版单百分比悬浮窗" /></figure>
        <figure><figcaption>当前实现</figcaption><div className="comparison-canvas"><div className="comparison-orb"><QuotaOrb snapshot={codex} language="zh-CN" descriptors={descriptorMap} onDrag={() => undefined} onHover={() => undefined} onToggleExpanded={() => undefined} /></div></div></figure>
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
