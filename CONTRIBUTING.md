# 参与开发

感谢参与 Tips。提交改动前请确保前端、Rust 后端和安全门禁都能通过。

## 开发环境

需要 Node.js 22+、pnpm 和稳定版 Rust，并安装当前平台对应的 Tauri 2 系统依赖。

```powershell
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` 会同时启动 Vite 开发服务器和 Tauri 桌面进程。需要隔离数据库或执行悬浮窗烟测时，请使用 [`docs/SMOKE_TESTING.md`](docs/SMOKE_TESTING.md) 中的步骤。

需要生成可直接运行、不依赖 Vite 的 release 可执行文件时使用：

```powershell
pnpm build:desktop
```

产物位于 `src-tauri/target/release/tips-desktop.exe`。不要用普通 `cargo build` 或 `cargo build --release` 生成桌面测试实例；Tauri 的生产资源协议由 Tauri CLI 构建流程启用，单独运行 Cargo 会继续指向 `devUrl`。

## 发布版本

本地需要安装包时执行 `pnpm build:release`，产物位于 `src-tauri/target/release/bundle/`。

GitHub 发布流程如下：

1. 将 `package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 的版本保持一致，同时更新相关锁文件。
2. 编写对应的 `docs/releases/v<版本>.md`，执行 `pnpm release:check` 和提交前检查。
3. 提交并推送源码，然后创建和推送同名版本标签，例如 `git tag -a v0.1.0 -m "Tips v0.1.0"`、`git push origin v0.1.0`。
4. `Release` workflow 会构建 Windows x64 NSIS、macOS Apple Silicon / Intel DMG、Linux x64 DEB / AppImage。四个构建任务和测试全部成功后，生成 SHA-256 校验文件并公开 Release。

`0.x` 版本和带预发布后缀的版本自动标记为 prerelease。失败时不会公开不完整的 Release；修复后可从 Actions 手动指定已存在标签重新运行。已公开的版本不会覆盖，请升级版本号再发布。

当前未配置 Windows 发行者证书或 Apple Developer notarization；macOS 包仅使用 ad-hoc 签名。macOS 透明悬浮窗启用了 Tauri `macos-private-api`，目前采用 DMG 分发，不适合 App Store 上架。各平台实际安装、凭据库、托盘和悬浮窗行为仍需桌面验收。

## 提交前检查

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

`pnpm build` 包含 TypeScript/Vue 检查、Vite 构建和安全基线扫描。安全设计与报告方式见 [`SECURITY.md`](SECURITY.md)。

## 源码导航

- 数据模型：`src-tauri/src/model.rs`
- SQLite migrations：`src-tauri/migrations/`
- 加密与持久化：`src-tauri/src/store.rs`
- MCP 协议服务与逐请求鉴权：`src-tauri/src/mcp_server.rs`
- MCP 专用 DTO：`src-tauri/src/mcp_model.rs`
- 独立窗口与置顶：`src-tauri/src/windowing.rs`
- 主面板：`src/components/MainShell.vue`
- 悬浮便签：`src/components/StickyNote.vue`
- 前端到 Tauri 的调用封装：`src/api.ts`

已实现的 MCP v1 与后续协调能力边界记录在 [`MCP_CONTRACT.md`](MCP_CONTRACT.md) v0 草案中。修改 MCP 时仍需遵守 [`SECURITY.md`](SECURITY.md) 的默认本地监听、最小授权和敏感内容隔离要求。
