# Security

Tips 以“本地优先、最小权限、所有持久化内容均视为不可信输入”为安全基线。本文件描述的是工程控制与验证边界，不代表第三方安全认证。

## 当前信任边界

- 便签正文和 TODO 清单项使用 XChaCha20-Poly1305 加密后写入 SQLite，主密钥保存在系统凭据库。
- 分组、标题、标签、颜色和时间等检索元数据仍是明文；不要把密码、令牌或恢复码写入这些字段。
- 临时秘密强制设置为敏感且拒绝 MCP，未指定到期时间时默认 24 小时过期。当前尚未实现自动清除任务。
- 本地同一操作系统账户下运行的恶意程序、截屏、键盘记录、进程内存读取和已解锁桌面不在本应用能够独立抵御的范围内。

## WebView 与 XSS 基线

- 用户内容只能通过 Vue 文本插值或表单值渲染，不得使用 `v-html`、`innerHTML`、`outerHTML`、`insertAdjacentHTML`、`document.write`、`eval`、`new Function`、`javascript:` URL、HTML data URL 或 iframe `srcdoc`。
- 若未来引入富文本，必须使用维护中的严格白名单净化器，并增加针对事件属性、危险 URL、SVG/MathML 和 DOM clobbering 的回归测试。
- 生产 CSP 仅允许应用自身资源和 Tauri IPC；脚本不允许 `unsafe-inline` 或 `unsafe-eval`。样式因 Vue 动态样式绑定暂时允许 `unsafe-inline`。
- WebView 导航只允许 Tauri 应用源；Debug 构建额外允许精确的 `http://localhost:1420` 开发源。

## 桌面权限基线

- capability 只开放事件监听/注销、窗口拖动和最小化。
- 不启用通用 `core:default`、窗口默认权限、WebView 默认权限或 opener 插件。
- 进度导入/导出通过 Rust 命令调用原生文件选择器，WebView 不接收任意路径文件读写权限；应用只处理用户在对话框中明确选择的文件。
- 新增 Tauri 插件、命令、文件访问、Shell、剪贴板或外部 URL 能力时，必须逐项声明权限并说明数据流。

## 便携进度文件

- 导出的 `tips-progress` JSON 是为了跨设备迁移而生成的明文交换格式，普通便签正文和 TODO 文本不再享有 SQLite 静态加密保护；界面与使用文档必须持续明确提示这一点。
- 导出在 SQL 选行阶段排除 `secret` 和敏感条目，且格式不包含 MCP 客户端、token、分组授权、服务设置或窗口状态。
- 导入将整个文件视为不可信输入：限制为 64 MiB、2,000 个分组、10,000 条内容和每个 TODO 200 个清单项，并校验未知字段、UUID、重复 ID、revision、时间戳、分组与状态一致性。
- 验证或写入任一步失败时回滚整次导入；同 revision 的分歧不会静默覆盖，本机 revision 较高及本机私密同 ID 条目会被保留。
- 导入正文和清单项会使用目标设备的主密钥重新加密。当前交换格式不传播删除，也不构成实时或云端同步协议。

## MCP 暴露基线

当前 MCP 服务遵守 [MCP_CONTRACT.md](MCP_CONTRACT.md) 的最小本机边界：

- 只绑定 `127.0.0.1`，不存在匿名回退、隐式 `0.0.0.0` 监听或已实现的 LAN 开关。
- 每个 HTTP 请求都必须携带 Bearer token；token 使用高熵随机值，只在创建时返回一次，持久化时仅保存 SHA-256 哈希，并与 transport 绑定。
- SDK 层校验精确的 loopback `Host` 和 `Origin`，请求体上限为 1 MiB，并以共享信号量限制同时处理的请求数；缺失 `Origin` 的非浏览器客户端仍可使用。
- 客户端身份从认证请求派生，不接受工具参数伪造 actor。读取和写入均检查活跃客户端、活跃分组授权及角色。
- MCP 使用独立 SQL 查询与 DTO；在解密前排除 Secret、敏感内容、`mcp_access = deny` 和无权分组，且永不返回桌面窗口与外观状态。
- 写操作使用 item/entry revision 防止静默覆盖，并在同一 SQLite 事务内记录 MCP 事件。
- MCP 写入的毫秒时间戳受统一范围校验；前端日期格式化也会对损坏或超范围的旧数据降级显示。
- 客户端签发、授权查询与撤销命令只接受主窗口发起的 IPC；悬浮便签窗口不能管理持久 MCP 凭据。
- 未来的 LAN 模式必须显式开启，并额外落实 TLS 或可信反向代理；当前版本遇到非回环配置会拒绝启动 MCP 服务，但不会拖垮桌面应用。

## 本地验证

```powershell
pnpm security:check
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
pnpm audit --prod
```

`pnpm build` 会强制先执行安全门禁。该门禁用于阻止已知危险 DOM sink、缺失 CSP、过宽 Tauri capability 和 opener 插件回归；它是纵深防御的一部分，不能替代代码审查、依赖审计与运行时测试。

## 漏洞报告

请优先通过 GitHub 的私有漏洞报告功能（若仓库已启用）或私下联系维护者。不要在公开 Issue 中粘贴真实密码、令牌、数据库、主密钥或包含个人内容的截图。
