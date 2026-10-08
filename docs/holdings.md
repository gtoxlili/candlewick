# 持仓窗口

`src/holdings/`，入口 `holdings.html`。目标是打开就看懂自己的钱现在怎么样，所以页面只回答四个问题：有多少、今天怎么了、钱在哪、有什么风险。布局和行情窗口同一套（见 [design.md](design.md)）。

## 结构

```
TitleBar   「持仓」 ……………………………………… ● 更新于 14:03:27   [🔑 管理 API Key]
┌ 主栏 ─────────────────────────────────────────┬ 侧栏 280px ────────┐
│ Hero      128,430.52 USDT  [+1.24% · +1,572.30 24h]│ [账户 | 合约 n]      │
│           BTC +1,320 · SOL +310 · 合约 +755 · 稳定币 26% │                    │
│ Allocation ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇● │ 账户：每个交易所的  │
│           ● BTC 50% ● ETH 17% … ● 稳定币 26%          │ 总额、24h、钱包分布 │
│ [按价值 | 按 24h 盈亏]                                 │ 合约：每个仓位一张卡│
│ AssetList（panel）资产 数量 价格 24h 价值 成本·盈亏    │ 盈亏、回报率、开仓/ │
│           …每行背后一条占比条…                          │ 标记、距强平的尺    │
│           还有 n 个不足 1 USDT 的资产                   │                    │
│ Tiles     24h 盈亏 | 加密资产 | 稳定币 | 合约浮动盈亏    │                    │
└───────────────────────────────────────────────┴────────────────────┘
```

| 文件 | 内容 |
|---|---|
| `HoldingsApp.tsx` | 状态与布局：取数、偏好（排序、是否显示小额、侧栏页签，存 localStorage）、Hero、Tiles、标题栏里的新鲜度点和截图时隐藏的标记、空状态 |
| `summary.ts` | 纯函数：分布（`allocation`）、序列色（`seriesColor`）、四个数字（`figures`）、说明行（`insight`）、列表排序与小额折叠（`listed`）、强平距离与保证金回报率 |
| `Allocation.tsx` | 分布条和图例；末端的点在数据新鲜时以 mint 色呼吸 |
| `AssetList.tsx` | 资产列表；「按价值」时占比条按份额、「按 24h 盈亏」时按变动大小并用涨跌色 |
| `Aside.tsx` | 侧栏的账户与仓位 |

## 数据

一切来自 Rust 的 `Portfolio`（`src/lib/api.ts`，Rust 侧 `portfolio.rs` 的 `Portfolio::of`）：

- `accounts[]`：每个交易所的 `total`、`change`、`wallets`、`updated`、`error`
- `assets[]`：跨交易所合并后的资产，带 `change`（24h 带来的 USDT 变动）、`stable`、`held[]`（在哪个交易所的哪个钱包有多少）
- `positions[]`：合约仓位，带 `exchange`、`exposure`、`change`

页面先 `get_portfolio`，再订阅 `portfolio` 事件；窗口打开期间 Rust 每 10 秒刷新余额和仓位，并常驻一条 ticker 订阅，价格每秒合并一次重新估值后推过来，所以总资产会跳。`updated` 是余额的时间，不随价格跳动变化；标题栏的点在最近一次更新 20 秒内呈 mint 色并呼吸，读取中 amber，某账户出错 red，超时 faint。

24h 口径、估值和合并规则见 [market-data-providers.md](market-data-providers.md) 的「持仓」一节。

## 截图时隐藏

「设置 → 持仓 → 截图时隐藏持仓」（`Settings::conceal_holdings`）打开后，持仓不进截图。持仓出现在三个地方，各用系统允许的办法：

| 地方 | 做法 | 代码 |
|---|---|---|
| 持仓窗口 | 建窗口时 `content_protected`，开关变化时对开着的窗口 `set_content_protected`。macOS 上是 `NSWindow.sharingType = .none`，Windows 上是 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` | `window.rs` 的 `build` 与 `conceal_holdings` |
| 下拉菜单和总资产的子菜单 | 菜单窗口由系统在每次打开时新建，没法事先标记，只能在它出现时标。macOS 在下拉菜单跟踪期间监听窗口遮挡状态的通知，弹出菜单层级的窗口一上屏就设 `sharingType`；Windows 在 `TrackPopupMenuEx` 期间挂本线程的 `EVENT_SYSTEM_MENUPOPUPSTART` 事件钩子，逐个设显示亲和性 | `platform/macos/shield.rs`、`platform/windows/menu.rs` 的 `Shield` |
| 菜单栏 / 任务栏上的总资产 | 状态项的窗口归菜单栏，行情条是 Explorer 任务栏的子窗口，都没法排除，所以屏幕上也显示 `总资产 ••••`，任何金额都是同样四个点；涨跌幅照常显示 | `bar.rs` 的 `MASK` |

没有交易所 Key 时菜单里没有持仓，下拉菜单照常能截。打开时持仓窗口标题栏多一个划掉的相机图标，说明截图里为什么少了这个窗口，点它打开设置。

### 各系统能做到的程度

调研于 2026-10-08。

- macOS：苹果没有阻止截屏的公开接口（DTS 在开发者论坛的答复，[thread 760234](https://developer.apple.com/forums/thread/760234)）。`sharingType = .none` 只在 CGWindow 一层生效；macOS 15 起，用 ScreenCaptureKit 的应用（QuickTime 录屏和多数会议软件）照样拍得到，Electron 的 `setContentProtection` 文档和 [tauri#14200](https://github.com/tauri-apps/tauri/issues/14200) 都写明了这一点。
- Windows：`WDA_EXCLUDEFROMCAPTURE` 从 Windows 10 2004 起支持，截图、录屏和屏幕共享里窗口直接消失；更早的版本退化为 `WDA_MONITOR`，拍到的是黑块。
- 菜单是出现之后才标记的，录屏可能留下菜单刚出现的一两帧。
- 挡不住对着屏幕拍照。

## 在 Chrome 里迭代

```sh
pnpm dev
# 浏览器打开 http://localhost:1420/holdings.html
#            http://localhost:1420/holdings.html?state=loading   首个账户还没读到
#            http://localhost:1420/holdings.html?state=empty     还没有 API Key
#            http://localhost:1420/holdings.html?lang=ja         日文界面（en、zh-CN、ja；默认跟浏览器）
```

没有 Tauri 时，各页面的 `main.tsx` 会在开发构建里加载 `src/dev/mock.ts`，用 `@tauri-apps/api/mocks` 的 `mockIPC` 回答 `get_settings`、`get_portfolio` 等命令，数据是两家交易所、十来项资产和两个仓位的固定样本。生产构建里这段是死代码，不会打进包。浅色 / 深色用浏览器 DevTools 的渲染面板模拟；窗口尺寸是 980×640，最小 800×500。

## 空状态与错误

- 还没有 API Key：居中一段说明和「去设置」。
- 账户还没读到：Hero 骨架、分布条空、列表面板空、侧栏显示该账户「正在读取…」。
- 某账户读取失败：标题栏的点变红、文字「币安读取失败」（悬停见原因）；侧栏该账户下方显示原因；其余账户照常。
