# Tips

一个本地优先的跨平台桌面便签，面向 Windows、macOS 和 Linux。它既可以留在桌面上，也可以通过受控的 MCP 接口成为多个本地 Agent 共享的便签与 TODO 表单。

## 下载安装

在 [GitHub Releases](https://github.com/SilenceLurker/Tips/releases) 下载对应平台的安装包。Windows 提供 x64 安装程序，macOS 分别提供 Apple Silicon / Intel DMG，Linux 提供 x64 DEB / AppImage。每版附带 `SHA256SUMS.txt`；当前 `0.x` 版本标记为预发布版，平台要求和功能边界见对应版本说明。

## 当前能力

- 便签、TODO、临时秘密三种条目。
- 每条内容都可打开为独立无边框窗口，并保存位置、尺寸、显示状态、置顶状态及背景/文字两层透明度。
- 浅色/黑金主题可全局切换并实时同步到所有窗口；悬浮窗外观面板可直接更换六种背景色。
- 界面支持跟随系统语言、简体中文和 English，主窗口、悬浮窗与原生托盘保持同步。
- TODO 是独立清单结构：主面板编辑器和悬浮窗都支持逐项新增、改名、删除与勾选，并实时同步进度。
- 清单全部勾选时父任务自动完成，取消任意一项时自动恢复；整单完成/恢复也会同步所有清单项。
- 主面板可将普通便签、TODO、归档状态与分组导出为便携 JSON，并按稳定 ID/revision 合并到另一台设备；私密条目、MCP 凭据和窗口设置不会导出。适合单设备接续工作，暂不能完整识别两端独立编辑产生的冲突。
- 所有类型支持持久化分组、归档、颜色、自由标签、到期时间和乐观锁版本；空分组会保留，主面板可新建、重命名和筛选分组，悬浮窗会显示所属分组。
- 主窗口关闭后隐藏到托盘；托盘菜单可恢复主面板或退出。
- 正文和 TODO 清单项均使用 XChaCha20-Poly1305 加密后写入 SQLite，32 字节主密钥放在系统凭据库。
- 临时秘密强制标记为敏感、默认拒绝 MCP，并在未指定时写入 24 小时后的过期时间。

## MCP 当前能力

- 应用启动后默认在 `http://127.0.0.1:4784/mcp` 提供 Streamable HTTP；当前版本只允许本机回环地址，尚不开放 LAN。
- 每个 Agent/终端使用独立 Bearer token。请在主面板的 MCP 管理入口创建客户端、指定初始 `group` 和角色；之后可为同一客户端追加分组或调整角色，无需重新签发 token。token 只显示一次，数据库仅保存其 SHA-256 哈希。
- `viewer` 可读，`contributor` 和 `coordinator` 可在授权分组内创建、编辑、移动及完成任务；移动时必须同时拥有源组和目标组写权限。新建分组必须由用户为该客户端显式开启“允许管理分组”，重命名还要求它是源组的 `coordinator`。
- 当前工具支持列出、新建和重命名分组，读取/创建/编辑/移动便签和 TODO，修改条目状态，以及勾选单个 TODO 清单项。
- MCP DTO 不包含窗口坐标、尺寸、置顶、透明度、颜色或主题；`secret`、敏感条目和显式拒绝 MCP 的条目在 SQL 选行阶段即被隔离。

`group` 是稳定任务空间，标签只负责自由描述。任务分配、租期认领、事件游标、stdio bridge 和显式 LAN 模式仍属于后续阶段，边界记录在 [`MCP_CONTRACT.md`](MCP_CONTRACT.md) v0 草案中。

实际创建凭据、连接客户端、工具参数语义和故障排查见 [`docs/MCP_USAGE.md`](docs/MCP_USAGE.md)。

## 数据结构

| 表 | 用途 | 关键边界 |
| --- | --- | --- |
| `item_groups` | 持久化分组目录 | 空分组、规范化 `group_key` 与创建/更新时间；`inbox` 不可重命名 |
| `items` | 便签/TODO/秘密本体 | 稳定 `group_key`、内容密文、24 字节 nonce、状态、敏感级别、MCP 权限、revision 和时间戳 |
| `todo_entries` | TODO 的规范化清单项 | 条目文本密文、稳定 ID、顺序、完成状态、独立 revision 和完成时间 |
| `item_windows` | 每条内容的桌面窗口状态 | 坐标、尺寸、屏幕、缩放、置顶、背景透明度、文字透明度和可见性 |
| `settings` | 全局设置 | JSON 值；当前保存外观主题、界面语言与 MCP 本机服务配置 |
| `mcp_clients` | MCP 客户端登记 | transport、token hash、最后使用时间和吊销时间 |
| `mcp_group_grants` | 客户端的分组授权 | `viewer` / `contributor` / `coordinator` 角色与吊销时间 |
| `task_events` | MCP 修改事件 | 分组、actor、item revision 与追加式事件 ID；读取游标工具尚未开放 |

数据库约束保证 `secret` 必须同时满足 `sensitive` 和 `mcp_access = deny`。更新操作使用 `revision` 防止两个窗口静默覆盖彼此的修改。

安全边界：正文会加密；分组、标题、颜色、标签和时间等检索元数据目前是明文。临时秘密的标题与分组不要写入真正的密码或令牌。过期时间目前是数据字段，还未实现自动清除任务。

跨设备手动迁移的范围、明文文件注意事项和冲突规则见 [`docs/PORTABLE_SYNC.md`](docs/PORTABLE_SYNC.md)。当前功能不会传播删除，也不是实时云同步。

## 安全基线

- Vue 中的用户内容仅按文本渲染；构建门禁会拒绝 `v-html`、`innerHTML`、`eval`、危险 URL 等常见 XSS sink。
- 生产 WebView 使用限制性 CSP、精确导航白名单和最小 Tauri capability；未启用 opener 插件或通用默认权限。
- `pnpm build` 会先执行 `pnpm security:check`。详细威胁模型、MCP 暴露要求和漏洞报告方式见 [`SECURITY.md`](SECURITY.md)。

本项目以 [MIT License](LICENSE) 发布。

## 开发与验证

开发环境、构建命令和源码导航见 [`CONTRIBUTING.md`](CONTRIBUTING.md)；隔离数据目录、悬浮窗烟测和独立 debug 构建说明见 [`docs/SMOKE_TESTING.md`](docs/SMOKE_TESTING.md)。

发布通过版本标签触发的 [Release workflow](.github/workflows/release.yml) 完成：版本校验、四平台构建和测试全部通过后，才公开安装包与校验文件。
