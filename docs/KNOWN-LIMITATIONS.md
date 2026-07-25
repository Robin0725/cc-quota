# 已知限制

- Codex 与 Claude 数据来自各自客户端使用的只读额度服务，字段或认证方式可能变化。
- Claude 额度读取当前依赖 macOS Keychain 中的 Claude Code 登录态，因此本改版定位为 macOS 菜单栏工具。
- macOS 可能询问是否允许读取 Claude Code 的 Keychain 项；拒绝后 Claude 会显示未登录/不可用。
- 当前本地发布包仅使用 ad hoc 签名，未使用 Apple Developer ID、未公证，macOS 仍可能触发 Gatekeeper；每个变化后的安装包也会获得新的代码身份。应用会在发现当前二进制的辅助功能授权无效时，自动清理自身旧记录并重新触发一次系统授权，但用户仍必须亲自打开 macOS 权限开关。
- 真实额度依赖服务端返回的窗口数据；应用不会根据本地 token 消耗自行估算额度。
- 只有服务端没有返回 5 小时窗口时才回退周额度；5 小时额度为 0% 时仍显示 0%。
- 透明 WebView 使用 Tauri `macOSPrivateApi`，不能提交 Mac App Store，适合 GitHub Releases 或直接分发。
- 公开分发前建议补齐 Developer ID 签名和 notarization。
