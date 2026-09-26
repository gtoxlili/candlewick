<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Candlewick 应用图标：一条发光的实时价格线，末端是一个脉动的圆点">
</p>

<h1 align="center">Candlewick</h1>

<p align="center">在 macOS 菜单栏里看加密货币和股票的实时价格，带实时行情图、盘口和成交。</p>

<p align="center">
  <a href="https://github.com/gtoxlili/candlewick/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/gtoxlili/candlewick"></a>
  <img alt="macOS 15 或更高，Apple 芯片" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <a href="LICENSE"><img alt="许可证：GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/candlewick"></a>
</p>

<p align="center"><a href="README.md">English</a></p>

Candlewick 是一个免费开源的 macOS 菜单栏应用，实时显示加密货币和股票的价格。加密货币来自币安公开行情，支持任意现货交易对，不需要账号或 key；美股、港股和 A 股来自长桥，用你自己的 OpenAPI 凭证。把一个标的固定在菜单栏上，点下拉菜单里的任意一个，就能打开随每一笔成交跳动的行情图。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="Candlewick 的行情窗口：实时更新的 BTC/USDT 1 分钟 K 线、24 小时最高最低价和成交量，以及带买卖力量对比的 20 档盘口" src="assets/chart-light.png">
</picture>

## 功能

- 菜单栏价格：固定的标的直接显示最新价，可附带名称和涨跌幅（上下两排、更紧凑），下拉菜单列出全部自选
- 加密货币和股票放在一起，最多 30 个：任意币安现货交易对，以及按代码添加的美股、港股和 A 股（AAPL、700、600519）
- 美股跟随盘前、盘后和夜盘交易，下拉菜单里标注当前时段
- 实时行情图：折线或 K 线，加密货币周期从 1 秒到 1 天，股票从 1 分钟到 1 周，随每一笔成交更新，由 [Liveline](https://github.com/benjitaylor/liveline) 做平滑动画；拖动回看历史，滚动或双指捏合缩放
- 盘口和买卖力量对比（加密货币 20 档，股票按长桥行情权限提供的档数），以及实时的最近成交
- 绿涨红跌或红涨绿跌，登录时启动
- Mac 或显示器休眠时暂停，恢复后自动重连

## 安装

从[最新版本](https://github.com/gtoxlili/candlewick/releases/latest)下载 `Candlewick_<版本>_aarch64.dmg`，打开后把 Candlewick 拖进「应用程序」。需要 macOS 15 或更高、Apple 芯片的 Mac。应用用 Developer ID 签名并经过苹果公证，macOS 可以正常打开。

Candlewick 只出现在菜单栏，没有 Dock 图标，设置从下拉菜单打开。

## 用长桥看股票

股票行情需要一个开通了 OpenAPI 的长桥账户：

1. 登录 [open.longbridge.com](https://open.longbridge.com/)，在用户中心复制 App Key、App Secret 和 Access Token。
2. 粘贴到「设置 → 长桥」并保存。Candlewick 会登录一次，列出你在各个市场的行情权限。
3. 在搜索框里按代码添加股票，比如 AAPL、700、600519。

凭证只保存在你的 Mac 上，位于 `~/Library/Application Support/com.influo.candlewick/credentials.json`，只有你的账户能读取。Access Token 到期前会自动续期。

## 常见问题

### 需要账号或 API key 吗？

看加密货币不需要：Candlewick 通过 WebSocket 和 REST 读取币安公开的现货行情，不下单。看股票需要你自己的长桥 OpenAPI 凭证（见上文），Candlewick 只用它读取行情。

### 能看哪些标的？

任意币安现货交易对，比如 BTC/USDT、ETH/USDT、ETH/BTC；接入长桥后还有美股、港股和 A 股（含 ETF）。最多同时 30 个。

### 在访问不了 binance.com 的网络下能用吗？

它跟随 macOS 的系统代理设置（HTTPS 和 SOCKS）。`binance.com` 连不上时，会改用币安在 `binance.vision` 上的公开行情域名。长桥在大陆网络下会自动使用它的大陆接入点。

### 占用多少内存和 CPU？

只挂在菜单栏、价格持续刷新时，内存约 19 MB，CPU 不到单核的 1%。行情窗口和设置窗口只在打开时存在。

### 会收集数据吗？

不会。没有任何统计上报；应用只和币安的行情接口通信，填写凭证后也会连接长桥的 OpenAPI。设置和凭证只保存在你的 Mac 上。

### 有英文界面或 Intel 版本吗？

暂时没有。界面是简体中文，发布的安装包只支持 Apple 芯片。

## 从源码构建

需要 macOS 15 或更高、Rust 1.98+、Node.js 与 pnpm。

```sh
pnpm install
pnpm tauri dev     # 开发
pnpm tauri build   # 产物在 src-tauri/target/release/bundle/
```

## 代码结构

- `src-tauri/src/`：Rust 侧，包括菜单栏、窗口与设置存储
- `src-tauri/src/market/`：统一接口下的行情数据，币安和长桥各有一个数据源实现
- `src/settings/`：设置窗口
- `src/chart/`：行情窗口，图表基于 [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`：给 Liveline 加上拖动和缩放，让它跟随涨跌配色，并在窗口处于后台时降低重绘帧率

基于 [Tauri 2](https://tauri.app)、Rust、React 19 和 Tailwind CSS 构建。

## 许可

[GPL-3.0](LICENSE)。本项目与币安、长桥均无关联，价格仅供参考，不构成投资建议。
