<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Coin Tray 应用图标：一条发光的实时价格线，末端是一个脉动的圆点">
</p>

<h1 align="center">Coin Tray</h1>

<p align="center">在 macOS 菜单栏里看币安实时币价，带实时行情图、盘口和成交。</p>

<p align="center">
  <a href="https://github.com/gtoxlili/coin-tray/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/gtoxlili/coin-tray"></a>
  <img alt="macOS 15 或更高，Apple 芯片" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <a href="LICENSE"><img alt="许可证：GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/coin-tray"></a>
</p>

<p align="center"><a href="README.md">English</a></p>

Coin Tray 是一个免费开源的 macOS 菜单栏应用，实时显示比特币、以太坊以及任意币安现货交易对的价格。把关心的交易对固定在菜单栏上，点一下就能打开随每一笔成交跳动的行情图。数据来自币安公开行情接口，不需要账号、API key，也不用注册。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="Coin Tray 的行情窗口：实时更新的 BTC/USDT 1 分钟 K 线、24 小时最高最低价和成交量，以及带买卖力量对比的 20 档盘口" src="assets/chart-light.png">
</picture>

## 功能

- 菜单栏价格：固定的交易对直接显示最新价，可附带币种名称和 24 小时涨跌幅，下拉菜单列出全部交易对
- 实时行情图：折线或 K 线，周期从 1 秒到 1 天，随每一笔成交更新，由 [Liveline](https://github.com/benjitaylor/liveline) 做平滑动画；拖动回看历史，滚动或双指捏合缩放
- 20 档盘口和买卖力量对比，以及实时的最近成交
- 支持任意币安现货交易对，最多 30 个，可以搜索、固定和排序
- 绿涨红跌或红涨绿跌，登录时启动
- Mac 或显示器休眠时暂停，恢复后自动重连

## 安装

从[最新版本](https://github.com/gtoxlili/coin-tray/releases/latest)下载 `Coin-Tray_<版本>_aarch64.dmg`，打开后把 Coin Tray 拖进「应用程序」。需要 macOS 15 或更高、Apple 芯片的 Mac。应用用 Developer ID 签名并经过苹果公证，macOS 可以正常打开。

Coin Tray 只出现在菜单栏，没有 Dock 图标，设置从下拉菜单打开。

## 常见问题

### 需要币安账号或 API key 吗？

不需要。Coin Tray 只通过 WebSocket 和 REST 读取币安公开的现货行情数据，不下单，也不需要任何密钥。

### 能看哪些币？

任意币安现货交易对，比如 BTC/USDT、ETH/USDT、SOL/USDT、ETH/BTC，最多同时 30 个。

### 在访问不了 binance.com 的网络下能用吗？

它跟随 macOS 的系统代理设置（HTTPS 和 SOCKS）。`binance.com` 连不上时，会改用币安在 `binance.vision` 上的公开行情域名。

### 占用多少内存和 CPU？

只挂在菜单栏、价格持续刷新时，内存约 19 MB，CPU 不到单核的 1%。行情窗口和设置窗口只在打开时存在。

### 会收集数据吗？

不会。没有任何统计上报，也没有账号；应用只和币安的行情接口通信，设置只保存在你的 Mac 上。

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

- `src-tauri/src/`：Rust 侧，包括菜单栏、行情连接、窗口与设置存储
- `src/settings/`：设置窗口
- `src/chart/`：行情窗口，图表基于 [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`：给 Liveline 加上拖动和缩放，让它跟随涨跌配色，并在窗口处于后台时降低重绘帧率

基于 [Tauri 2](https://tauri.app)、Rust、React 19 和 Tailwind CSS 构建。

## 许可

[GPL-3.0](LICENSE)。本项目与币安无关联，价格仅供参考，不构成投资建议。
