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
| `HoldingsApp.tsx` | 状态与布局：取数、偏好（排序、是否显示小额、侧栏页签，存 localStorage）、Hero、Tiles、标题栏里的新鲜度点、空状态 |
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

## 在 Chrome 里迭代

```sh
pnpm dev
# 浏览器打开 http://localhost:1420/holdings.html
#            http://localhost:1420/holdings.html?state=loading   首个账户还没读到
#            http://localhost:1420/holdings.html?state=empty     还没有 API Key
```

没有 Tauri 时，各页面的 `main.tsx` 会在开发构建里加载 `src/dev/mock.ts`，用 `@tauri-apps/api/mocks` 的 `mockIPC` 回答 `get_settings`、`get_portfolio` 等命令，数据是两家交易所、十来项资产和两个仓位的固定样本。生产构建里这段是死代码，不会打进包。浅色 / 深色用浏览器 DevTools 的渲染面板模拟；窗口尺寸是 980×640，最小 800×500。

## 空状态与错误

- 还没有 API Key：居中一段说明和「去设置」。
- 账户还没读到：Hero 骨架、分布条空、列表面板空、侧栏显示该账户「正在读取…」。
- 某账户读取失败：标题栏的点变红、文字「币安读取失败」（悬停见原因）；侧栏该账户下方显示原因；其余账户照常。
