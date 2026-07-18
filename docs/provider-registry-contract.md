# Provider 注册表契约 v1

**这是施工的唯一事实源。** 两步改造(先抽象、后接入)都以本文为准,偏离要先改本文。

## 目标

加一个新 AI provider = **写一个描述 + 一个适配器 + 注册一行**,别处零改动。

硬验收指标:接入 kimicode 的那个 commit,diff 必须只包含
- 一个新文件 `src-tauri/src/providers/kimicode.rs`
- 一处注册(registry 数组里加一行)
- 前端 mock 数据加一条

**若需要回头改公共逻辑,说明抽象失败,回炉重做。**

---

## 一、Rust 侧

### 1.1 ProviderDescriptor(声明式描述)

```rust
pub struct ProviderDescriptor {
    /// 稳定 id,前后端与配置文件共用。禁止改动已发布的值。
    pub id: &'static str,                    // "codex" | "claude" | "kimicode"
    pub display_name: &'static str,          // "Codex" / "Claude" / "Kimi Code"
    /// 菜单栏胶囊上的两字母标记
    pub abbreviation: &'static str,          // "CX" / "CL" / "KM"
    /// 托盘位图色板
    pub palette: CapsulePalette,
    /// 前端用的强调色(hex),与 palette 同源,见 §三
    pub accent_hex: &'static str,
    /// 前台 app 归属判断:bundle id 或进程名里命中任一子串即算该 provider。
    /// 全部小写,调用方负责小写化输入。
    pub focus_hints: &'static [&'static str],
}
```

### 1.2 ProviderAdapter(行为)

```rust
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> &'static ProviderDescriptor;

    /// 本地是否存在登录态(凭证文件/Keychain 项)。
    /// 用于"发现即启用":返回 false 的 provider 不参与抓取、不出现在 UI。
    /// 只做存在性检查,不得发网络请求、不得解密/输出凭证内容。
    fn is_configured(&self) -> bool;

    /// 该 provider 的 CLI 在活动时会写入的目录(会话日志等)。
    /// 用于判断"用户此刻在用哪个" —— 见 §六。返回空切片表示该 provider
    /// 不提供活动信号(它就永远不会被选为活动 provider,但仍正常显示配额)。
    fn activity_paths(&self) -> Vec<PathBuf>;

    /// 取配额。**永不 panic、永不返回 Err** —— 失败一律返回
    /// ProviderSnapshot::failure_for(id, display_name, status, message),
    /// status 取 "signed_out" | "unavailable"。
    async fn fetch_snapshot(&self, client: &reqwest::Client) -> ProviderSnapshot;
}
```

### 1.3 注册表

```rust
/// 全部已知 provider,顺序即 UI 展示顺序。
pub fn all() -> &'static [&'static dyn ProviderAdapter];

/// 本地有登录态的 provider(all() 里 is_configured() 为真的子集)。
pub fn configured() -> Vec<&'static dyn ProviderAdapter>;

/// 前台 app 未命中任何 focus_hints 时归给谁。
/// 保持现有行为 = "claude"。若该 provider 未登录,则取 configured() 的第一个。
pub const DEFAULT_FOCUS_PROVIDER: &str = "claude";
```

### 1.4 必须改成表驱动的调用点

| 现状 | 改后 |
|---|---|
| `lib.rs:908` `tokio::join!` 写死两路 | 对 `configured()` 用 `futures::future::join_all` |
| `lib.rs:830` `unavailable_snapshots()` 两元素 vec | 按 `all()` 生成 |
| `lib.rs:107` `classify_frontmost_application` 二分类 | 遍历 `all()` 匹配 `focus_hints`,兜底 `DEFAULT_FOCUS_PROVIDER` |
| `lib.rs:625` `tray_icon_rgba(4 个写死参数)` | 收 `&[ProviderSnapshot]`,按数量算胶囊坐标 |
| `lib.rs:672/716/1131` 两个具名 MenuItem | 按 `configured()` 循环生成 |
| `models.rs:103` pinned_provider 白名单写死 | 校验改成查 `all()` 的 id |

### 1.5 托盘位图布局(唯一需要算数的地方)

现在:`TRAY_ICON_WIDTH=172`(@2x),两胶囊坐标是常量。

改后按 provider 数量算:
```
capsule_width  = 80  (@2x, 即 40pt)
capsule_gap    = 12  (@2x)
icon_width     = n * capsule_width + (n - 1) * capsule_gap
```
n=2 时结果必须与现有 172 一致(80*2+12=172 ✓),**以此验证重构没改变现有观感**。

高度不变。n 的上限不设硬编码,但 n>4 时记一条 warn 日志(菜单栏空间提示)。

---

## 二、前端侧

### 2.1 类型

`ProviderId` 从字面量联合类型改为 `string`。前端**不得**再对 provider id 做穷举判断(`=== "codex"` 之类一律删除)。

### 2.2 描述从 Rust 单向下发(关键决定)

新增 tauri command:

```
get_provider_descriptors() -> Vec<ProviderDescriptorDto>
```

```ts
interface ProviderDescriptorDto {
  id: string;
  displayName: string;
  abbreviation: string;   // 取代 QuotaCard.tsx:22 的 "CX"/"CL" 三元硬编码
  accentHex: string;      // 取代 CSS 里的 per-provider class
}
```

**理由:颜色和缩写现在在 Rust(托盘位图)和 CSS 里各定义一份,是漂移源头。**
改成 Rust 单一定义、前端消费,加 provider 时不必两边同步改。

### 2.3 CSS 改造

删除全部 `--codex` / `--claude` 后缀的 provider 专属 class
(`.quota-orb--codex`、`.detail-provider--claude` 等,styles.css:38-46/71/135-136/139-140/148-149)。

改用 CSS 自定义属性,由组件按 descriptor 注入行内样式:
```tsx
<div className="detail-provider" style={{ ["--provider-accent" as string]: d.accentHex }}>
```
CSS 里一律引用 `var(--provider-accent)`。

**这同时修掉"Codex 蓝是隐式默认值"的坑** —— 不再有 provider 继承别人的颜色。

severity 配色(healthy/caution/critical, `format.ts:31`)与 provider 正交,保持不变。

### 2.4 必须修的截断

- `QuotaCard.tsx:197` `snapshots.slice(0, 2)` → **删掉 slice**,渲染全部。
- `styles.css:132` `grid-template-rows: repeat(2, ...)` → `repeat(auto-fit, ...)` 或按数量算。
- 详情面板 320×320 固定尺寸:三个卡片先压缩行高适配;**超过 4 个时面板内容区改为纵向滚动**,窗口尺寸不变。
- `App.tsx:10` `PROVIDER_ORDER` 常量删除,顺序改用 descriptor 下发顺序。

---

## 三、色板单一来源

每个 provider 的颜色**只在 `ProviderDescriptor` 里定义一次**:
- `palette`(RGBA 四元组)供托盘位图使用
- `accent_hex` 供前端使用,必须是 `palette.fill_bottom` 的 hex 表示

现有值保持不变(重构后观感必须一致):
- Codex:`fill_top [25,55,82]` / `fill_bottom [47,111,237]` → accent `#2f6fed`
- Claude:`fill_top [91,49,37]` / `fill_bottom [184,90,58]` → accent `#b85a3a`
- kimicode:第二步新增,取一个与蓝/橙都拉得开的色相(建议紫,待定)

---

## 四、kimicode 适配器规格(第二步用)

### 4.1 认证

读 `~/.kimi-code/credentials/kimi-code.json`(明文 JSON,**不走 Keychain**):
```json
{ "access_token": "...", "refresh_token": "...", "expires_at": <unix秒>, "token_type": "Bearer" }
```
- `is_configured()` = 该文件存在且能解析出 `access_token`。
- **app 不参与 token 续期(2026-07-19 拍板,推翻原刷新方案)。** 只读 `access_token`,
  `refresh_token` 一律不碰、凭证文件一律不写。`expires_at` 已过期或 60 秒内到期 →
  直接返回失败(`unavailable`,文案说明"CLI 下次运行时会自己续期"),走 §附录 的保底显示。

  理由:access token 寿命只有 15 分钟,Kimi CLI 在用户使用期间自己续期并写文件 ——
  用户在用 kimi 时 token 永远是新的,而配额数字恰恰只在那时重要。反过来,若服务端的
  refresh_token 是一次性轮换的,app 刷新后不回写文件就会**让用户的 CLI 登录失效**;
  回写又要与 CLI 抢文件。收益(不用 kimi 时也能看实时数)远小于搞坏别人登录态的风险。

### 4.2 取数

```
GET https://api.kimi.com/coding/v1/usages
Authorization: Bearer <access_token>
Accept: application/json
```

**已实测通过(HTTP 200)的真实响应形状** —— 注意与网上文档的差异:

```json
{
  "usage":  { "limit": "100", "remaining": "100", "resetTime": "2026-07-25T16:06:49Z" },
  "limits": [ { "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": { "limit": "100", "used": "1", "remaining": "99",
                            "resetTime": "2026-07-18T21:06:49Z" } } ],
  "totalQuota": { "limit": "100", "remaining": "99" },
  "user": { "membership": { "level": "LEVEL_INTERMEDIATE" } },
  "subType": "TYPE_PURCHASE"
}
```

解析要点(**踩过的坑,别想当然**):
1. **数字是字符串**,`"100"` 不是 `100`。解析要容忍字符串和数字两种。
2. 顶层 `usage` 给的是 **`remaining`**,不是 `used`。剩余百分比 = `remaining / limit * 100`。
   若某版本只给 `used`,则 `remaining = limit - used`。两种都要兼容。
3. 字段名是 **`resetTime`**,不是 `resetAt` / `reset_at`。仍建议兼容这几种别名。
4. `timeUnit` 带枚举前缀:**`"TIME_UNIT_MINUTE"`**,不是 `"MINUTE"`。解析时剥前缀。
5. `window.duration` + `timeUnit` 换算成秒:`300 MINUTE` = 18000 秒 = 5 小时。
6. `boosterWallet`(付费超额包)本账号未返回,**当作可选字段**,缺失不算错误。

### 4.3 映射到 ProviderSnapshot

- `limits[]` 里 window 时长 ≈ 5 小时的那条 → `short_window`
- 顶层 `usage`(周)→ `weekly_window`
- `scoped_windows` → 留空(kimi 无按模型分桶)
- `plan` → 由 `user.membership.level` 去掉 `LEVEL_` 前缀后格式化
- 沿用现有"5H 优先、缺了退化到周并打 W 标"的公共逻辑,**不在适配器里重复实现**

### 4.4 focus_hints

`["kimi"]` —— 需覆盖 Kimi Code 的终端/GUI。注意别误伤:hints 匹配在 `all()` 上按注册顺序取首个命中。

---

## 六、活动 provider 判定(悬浮窗显示谁)

### 6.1 为什么不用前台 app

原方案按前台 macOS 应用判断归属。该信号在真实用法下无解:用户在**同一个终端**里跑 Claude、Codex、Kimi,
前台 app 永远是终端本身,三者无法区分。

改用**活动信号**:每个 CLI 干活时都在自己的会话目录里写文件,谁刚写过谁就是用户正在用的。
跨终端、跨 tmux 都成立,且不需要辅助功能权限。

### 6.2 必须用 FSEvents,不许轮询遍历

实测(本机,2026-07-19):三个会话目录共约 2800 个文件,全量遍历一次 **25ms**。
按 2 秒轮询 = 持续占用 **1.25% CPU**,且该开销**正比于历史会话文件总数**——用得越久越慢。
一个越用越卡的设计,起点再低也是错的。

因此:用 `notify` crate(底层走 macOS FSEvents)监听目录变化。开销只与"发生了多少变化"有关,
与"存了多少文件"无关;闲置时不唤醒,变化时即时响应。

**兜底不得退化成全目录遍历。** 监听建立失败时,退回"只 stat 当前那一个最新会话文件"
(单文件 stat 是微秒级)。若连兜底也不可用,则退回 §1.3 的 `classify_focus`(前台 app),
即恢复旧行为,而不是给出错误答案。

### 6.3 选择规则

**只实现这一条:显示最近有活动的 provider。**

- 各 provider 维护一个 `last_activity: Option<Instant/SystemTime>`,取其最大者为活动 provider。
- 从未观测到任何活动(如刚装好)→ 回落到 §1.3 `classify_focus`。
- 活动 provider 未登录 / 不在 `configured()` 中 → 跳过,取次新的。

**不要实现"低配额抢占显示"。** 那是提过但用户尚未拍板的第二条规则,未经同意不得加入。

## 五、纪律

- 第一步是**纯重构**:现有测试必须全绿,且托盘位图在 n=2 时逐像素与改前一致。
- 测试里"恰好两个 provider"的断言(`QuotaCard.test.tsx:128`、`lib.rs` tray_icon_tests)改为**参数化**,不要只把 2 改成 3。
- 任何情况下不得把 token/凭证内容写进日志、错误信息或测试快照。

---

## 附录:取数失败时的保底显示(2026-07-19 补,公共逻辑)

失败一律保留上次成功的数值并标记 `stale`,**不按 status 区别对待**。

- `merge_snapshots`(lib.rs)与 `mergeSnapshots`(src/lib/snapshots.ts):只有 `ok` 直接替换;
  其余任何 status(含 `signed_out`)都回落到 30 分钟内的上次成功读数,状态改 `stale`,
  并把失败 message 带上。
- 从未成功过(没有可回落的读数)→ 原样上报失败,不编数字。
- **不得按 status 判断"这次失败是不是永久性的"。** 短寿命 token(kimi 15 分钟)过期时
  会报 `signed_out`,而 CLI 马上就会续期 —— 早先按 status 清空数值的写法,正是 0.5.0 里
  kimi 胶囊每刻钟整个消失的原因。允许显示多久由 30 分钟的 `MAX_STALE_SECONDS` 界定,而不是 status。
