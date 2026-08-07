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
- 新增 Tauri 插件、命令、文件访问、Shell、剪贴板或外部 URL 能力时，必须逐项声明权限并说明数据流。

## MCP 暴露基线

MCP 服务端尚未实现。实现时必须遵守 [MCP_CONTRACT.md](MCP_CONTRACT.md)：

- 默认仅绑定 `127.0.0.1`，不存在匿名回退或隐式 `0.0.0.0` 监听。
- LAN 模式必须显式开启，并使用 bearer token 哈希、按组授权、Origin 校验和 TLS。
- 客户端身份从认证连接派生，不接受调用方伪造 actor、assignee 或 claimant。
- Secret 永远不进入 MCP；所有写操作使用 revision/claim revision 防止静默覆盖。

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
