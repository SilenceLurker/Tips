# Tips

一个本地优先的跨平台桌面便签，面向 Windows、macOS 和 Linux。当前阶段先把数据边界、主面板和真实悬浮窗做扎实，MCP 传输层留到下一阶段。

## 当前能力

- 便签、TODO、临时秘密三种条目。
- 每条内容都可打开为独立无边框窗口，并保存位置、尺寸、显示状态、置顶状态及背景/文字两层透明度。
- 浅色/黑金主题可全局切换并实时同步到所有窗口；悬浮窗外观面板可直接更换六种背景色。
- TODO 是独立清单结构：主面板编辑器和悬浮窗都支持逐项新增、改名、删除与勾选，并实时同步进度。
- 清单全部勾选时父任务自动完成，取消任意一项时自动恢复；整单完成/恢复也会同步所有清单项。
- 所有类型支持稳定分组、归档、颜色、自由标签、到期时间和乐观锁版本；主面板可按分组筛选，悬浮窗会显示所属分组。
- 主窗口关闭后隐藏到托盘；托盘菜单可恢复主面板或退出。
- 正文和 TODO 清单项均使用 XChaCha20-Poly1305 加密后写入 SQLite，32 字节主密钥放在系统凭据库。
- 临时秘密强制标记为敏感、默认拒绝 MCP，并在未指定时写入 24 小时后的过期时间。

MCP 服务端尚未实现。数据库已经预留客户端、transport、token hash 和 scopes 字段；当前应用不会监听本机或局域网端口。

下一阶段 MCP 的分组、客户端身份、任务分配、租期认领、事件游标和传输安全边界已经冻结在 [`MCP_CONTRACT.md`](MCP_CONTRACT.md)。`group` 是稳定任务空间，标签只负责自由描述；未来的 MCP 默认只绑定 `127.0.0.1`，局域网必须显式开启。

## 数据结构

| 表 | 用途 | 关键边界 |
| --- | --- | --- |
| `items` | 便签/TODO/秘密本体 | 稳定 `group_key`、内容密文、24 字节 nonce、状态、敏感级别、MCP 权限、revision 和时间戳 |
| `todo_entries` | TODO 的规范化清单项 | 条目文本密文、稳定 ID、顺序、完成状态、独立 revision 和完成时间 |
| `item_windows` | 每条内容的桌面窗口状态 | 坐标、尺寸、屏幕、缩放、置顶、背景透明度、文字透明度和可见性 |
| `settings` | 全局设置 | JSON 值；当前保存外观主题 |
| `mcp_clients` | 下一阶段的 MCP 客户端登记 | stdio/loopback/LAN transport、token hash、scopes 和吊销时间 |

数据库约束保证 `secret` 必须同时满足 `sensitive` 和 `mcp_access = deny`。更新操作使用 `revision` 防止两个窗口静默覆盖彼此的修改。

安全边界：正文会加密；分组、标题、颜色、标签和时间等检索元数据目前是明文。临时秘密的标题与分组不要写入真正的密码或令牌。过期时间目前是数据字段，还未实现自动清除任务。

## 安全基线

- Vue 中的用户内容仅按文本渲染；构建门禁会拒绝 `v-html`、`innerHTML`、`eval`、危险 URL 等常见 XSS sink。
- 生产 WebView 使用限制性 CSP、精确导航白名单和最小 Tauri capability；未启用 opener 插件或通用默认权限。
- `pnpm build` 会先执行 `pnpm security:check`。详细威胁模型、MCP 暴露要求和漏洞报告方式见 [`SECURITY.md`](SECURITY.md)。

本项目以 [MIT License](LICENSE) 发布。

核心定义：

- 数据模型：`src-tauri/src/model.rs`
- SQLite migration：`src-tauri/migrations/001_initial.sql` 至 `src-tauri/migrations/004_item_groups.sql`
- 加密与持久化：`src-tauri/src/store.rs`
- 独立窗口与置顶：`src-tauri/src/windowing.rs`
- 主面板：`src/components/MainShell.vue`
- 悬浮便签：`src/components/StickyNote.vue`

## 本地开发

需要 Node.js 20+、pnpm 和稳定版 Rust。

```powershell
pnpm install
pnpm tauri dev
```

构建前检查：

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

## 调试烟测

Debug 构建支持两个显式环境变量：`TIPS_DATA_DIR` 可将 SQLite 隔离到临时目录；`TIPS_SMOKE_FLOAT=1` 会切到黑金主题，并在 `smoke/runtime` 分组创建一条含三项清单、背景 65%、文字 85% 的置顶 TODO。Release 构建不会启用这两个调试入口。

热重载开发必须通过 Tauri CLI 启动，它会同时启动 Vite：

```powershell
$env:TIPS_DATA_DIR = "C:\tmp\tips-dev"
$env:TIPS_SMOKE_FLOAT = "1"
pnpm tauri dev
```

需要不依赖 localhost 的独立 debug 可执行文件时，必须先让 Tauri CLI 构建并嵌入 `dist`：

```powershell
pnpm tauri build --debug --no-bundle
$env:TIPS_DATA_DIR = "C:\tmp\tips-standalone"
$env:TIPS_SMOKE_FLOAT = "1"
.\src-tauri\target\debug\tips-desktop.exe
```

不要直接用普通 `cargo build` 产物做独立界面烟测。该 debug 产物使用 `devUrl`，没有同时运行 Vite 时会显示 `localhost:1420` 拒绝连接。
