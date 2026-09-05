# 调试烟测

本页记录开发者专用的隔离数据和桌面悬浮窗烟测方式。以下入口仅在 debug 构建中生效，release 构建不会读取它们。

## 调试环境变量

- `TIPS_DATA_DIR`：将 SQLite 和相关运行数据写入指定目录，避免污染日常使用的数据。
- `TIPS_SMOKE_FLOAT=1`：切换到黑金主题，并在 `smoke/runtime` 分组创建一条置顶 TODO。该条目包含三项清单，背景透明度为 65%，文字透明度为 85%。

请为每轮测试使用专用目录，测试结束后只清理自己明确创建的目录。

## 热重载烟测

热重载开发必须通过 Tauri CLI 启动，由它同时启动 Vite：

```powershell
$env:TIPS_DATA_DIR = "C:\tmp\tips-dev"
$env:TIPS_SMOKE_FLOAT = "1"
pnpm tauri dev
```

## 独立 debug 可执行文件

需要测试不依赖 localhost 的 debug 可执行文件时，先让 Tauri CLI 构建并嵌入 `dist`：

```powershell
pnpm tauri build --debug --no-bundle
$env:TIPS_DATA_DIR = "C:\tmp\tips-standalone"
$env:TIPS_SMOKE_FLOAT = "1"
.\src-tauri\target\debug\tips-desktop.exe
```

不要直接使用普通 `cargo build` 或 `cargo build --release` 的产物做独立界面烟测。Tauri 的生产资源协议由 Tauri CLI 构建流程启用；单独运行 Cargo 会继续使用 `devUrl`，没有同时运行 Vite 时就会停在空白页或显示 `localhost:1420` 拒绝连接。

## 最低检查项

1. 主面板和悬浮窗均能打开，且交互时进程持续响应。
2. 黑金主题、置顶状态、背景透明度和文字透明度正确显示。
3. TODO 清单可新增、编辑、删除和切换完成状态，重启后数据仍然存在。
4. 测试期间只读写 `TIPS_DATA_DIR` 指向的隔离目录。
