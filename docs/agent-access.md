# AI 助手接入

设置 → 通用 →「AI 助手接入」打开后，应用做两件事：在本机开一个只读 HTTP 接口，并把一份 skill 写进本机 AI 编程助手的 skill 目录，告诉它们接口怎么用。关掉时接口停止，skill 删除。应用启动时按设置自动恢复。

代码在 `src-tauri/src/agent/`：`mod.rs` 管开关、`api.rs` 是接口（axum）、`skill.rs` 管安装，接口契约就是 skill 本身 `SKILL.md`（安装时填入本机的地址和令牌路径）。

## 接口

- 只监听 `127.0.0.1`，默认端口 52733，被占用时换一个空闲端口（skill 每次启动都会重写，地址跟着变）。
- 只读：行情、持仓、自选、搜索；没有下单、划转或改设置的接口。
- 可以查任意交易对或股票，不限于自选：K 线、成交和实时流只用到来源和代码。
- `/v1/portfolio` 先刷新超过 15 秒的持仓（最多等 10 秒），等待条件是每个账户要么数据够新，要么在请求之后完成过一次刷新（`portfolio::Account.checked`，成功失败都算），所以读取失败时不会白等到超时。
- `/v1/market` 借用行情窗口的实时流，拿到第一份统计和盘口就断开。

## 安全

- 令牌：32 字节随机数，存在配置目录的 `agent-token`（0600）。skill 里的命令用 `$(cat …)` 读取，令牌不进对话。比较用 HMAC 校验，耗时与猜对多少无关。它挡的是同一台机器上的其他用户。
- 网页：浏览器请求带 `Origin`，一律拒绝；DNS 重绑定的页面会把自己的域名放进 `Host`，所以 `Host` 只接受 `127.0.0.1:端口` 和 `localhost:端口`。只监听回环地址，Windows 不会弹防火墙提示。

## skill 装在哪

调研于 2026-10-01（Claude Code、Codex 0.159、OpenCode 1.18）。

| 助手 | 读的目录 | 写到哪 |
|---|---|---|
| Claude Code | `~/.claude/skills`（`CLAUDE_CONFIG_DIR`） | 这里 |
| OpenCode | `~/.config/opencode/skills`，也读 `~/.claude/skills` 和 `~/.agents/skills` | 有 Claude Code 时不另写，否则写 `~/.config/opencode/skills` |
| Codex | `$CODEX_HOME/skills`（默认 `~/.codex/skills`）；新文档另有 `~/.agents/skills` | `~/.codex/skills` |

不写 `~/.agents/skills`：OpenCode 也读它，会和 `~/.claude/skills` 里的同名 skill 重复。只写进已经存在的助手目录，不替没装的助手建目录。每份 skill 带一个 `.candlewick` 标记文件，卸载时只删有标记的，用户自己的同名 skill 不会被覆盖或删除。

skill 按 harness 设计原则写：只给事实（接口、字段、单位）和数据语义（估值口径、`change` 怎么算、数据新鲜度），不写投资策略，怎么分析交给助手。
