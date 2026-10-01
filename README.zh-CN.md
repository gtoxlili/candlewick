<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Candlewick 应用图标：一条发光的实时价格线，末端是一个脉动的圆点">
</p>

<h1 align="center">Candlewick</h1>

<p align="center">在 macOS 菜单栏或 Windows 任务栏里看加密货币和股票的实时价格，带实时行情图、盘口和成交。</p>

<p align="center">
  <a href="https://github.com/gtoxlili/candlewick/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/gtoxlili/candlewick"></a>
  <img alt="macOS 15 或更高，Apple 芯片" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <img alt="Windows 10 或 11" src="https://img.shields.io/badge/Windows-10%20%C2%B7%2011-0078d4">
  <a href="LICENSE"><img alt="许可证：GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/candlewick"></a>
</p>

<p align="center"><a href="README.md">English</a></p>

加密货币用币安、Bybit 或 OKX 的公开行情，不需要账号；美股、港股和 A 股来自长桥，需要你自己的 OpenAPI 凭证。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="Candlewick 的行情窗口：实时更新的 BTC/USDT 1 分钟 K 线、24 小时最高最低价和成交量，以及带买卖力量对比的 20 档盘口" src="assets/chart-light.png">
</picture>

## 功能

- 菜单栏或任务栏常驻一个标的，下拉菜单列出全部自选，最多 30 个
- 加密货币：币安、Bybit、OKX 任选一家，任意现货交易对
- 股票：美股（含盘前、盘后、夜盘）、港股、A 股，按代码添加
- 行情图随每笔成交跳动，带盘口和最近成交
- 填上交易所的 API Key，就能看总资产和各账户持仓
- 打开「AI 助手接入」，Claude Code、Codex 和 OpenCode 就能读取你的行情和持仓，陪你分析
- 红涨绿跌可选，开机启动，自动更新

## 安装

**macOS**：下载[最新版本](https://github.com/gtoxlili/candlewick/releases/latest)里的 `Candlewick_<版本>_aarch64.dmg`，把 Candlewick 拖进「应用程序」。需要 macOS 15 或更高、Apple 芯片。它只在菜单栏里，没有 Dock 图标。

**Windows**：下载并运行 `Candlewick_<版本>_x64-setup.exe`，需要 Windows 10 或 11。SmartScreen 拦下时，点「更多信息」再点「仍要运行」。价格显示在任务栏时钟旁边。

## 看股票

1. 在 [open.longbridge.com](https://open.longbridge.com/) 的用户中心复制 App Key、App Secret 和 Access Token。
2. 粘贴到「设置 → 长桥」。
3. 在搜索框里输入代码添加，比如 AAPL、700、600519。

## 看持仓

1. 在交易所创建 API Key，只需要读取权限；OKX 还要设一个 Passphrase。
2. 粘贴到「设置 → 持仓」。
3. 点下拉菜单最上面的「总资产」，查看各账户的持仓。

## 常见问题

**访问不了交易所的网站也能用吗？** 能。它走系统代理，主地址不通时会自动换备用地址。

**占用多少资源？** 在 Mac 上约 19 MB 内存，CPU 不到单核的 1%。

**会收集数据吗？** 不会。它只连你用到的交易所和长桥，以及 GitHub（检查更新，可在设置里关掉）。凭证和 Key 只保存在本机，Candlewick 从不下单。打开「AI 助手接入」后，本机的 AI 助手能读取数据，别的电脑和网页都读不到。

**有英文界面或 Intel 版本吗？** 暂时没有。

## 从源码构建

需要 Rust 1.98+、Node.js 和 pnpm。在 Mac 上还需要 macOS 15 或更高，在 Windows 上还需要 Visual Studio 的 C++ 生成工具。

```sh
pnpm install
pnpm tauri dev     # 开发
pnpm tauri build   # 安装包在 src-tauri/target/release/bundle/
```

Windows 版是怎么实现的，见 [docs/windows.md](docs/windows.md)。main 上每次改动应用的推送都会自动发布新版本，流程见 [docs/release.md](docs/release.md)。

## 代码结构

- `src-tauri/src/`：Rust 侧，包括菜单栏和任务栏显示的内容、窗口与设置
- `src-tauri/src/platform/`：macOS 和 Windows 之间不同的部分
- `src-tauri/src/market/`：行情与持仓，各数据源的实现。接入说明见 [docs/market-data-providers.md](docs/market-data-providers.md)
- `src-tauri/src/portfolio.rs`：持仓估值
- `src-tauri/src/agent/`：AI 助手接入，本机只读接口和 skill，见 [docs/agent-access.md](docs/agent-access.md)
- `src/settings/`、`src/chart/` 与 `src/holdings/`：设置、行情和持仓窗口，图表基于 [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`：给 Liveline 加上拖动、缩放和涨跌配色

基于 [Tauri 2](https://tauri.app)、Rust、React 19 和 Tailwind CSS 构建。

## 许可

[GPL-3.0](LICENSE)。本项目与币安、Bybit、OKX、长桥均无关联，价格仅供参考，不构成投资建议。
