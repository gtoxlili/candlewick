<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Candlewick app icon: a glowing live price line ending in a pulsing dot">
</p>

<h1 align="center">Candlewick</h1>

<p align="center">Live crypto and stock prices in the macOS menu bar and the Windows taskbar, with a real-time chart, order book and trades.</p>

<p align="center">
  <a href="https://github.com/gtoxlili/candlewick/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/gtoxlili/candlewick"></a>
  <img alt="macOS 15 or later on Apple silicon" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <img alt="Windows 10 or 11" src="https://img.shields.io/badge/Windows-10%20%C2%B7%2011-0078d4">
  <a href="LICENSE"><img alt="License: GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/candlewick"></a>
</p>

<p align="center"><a href="README.zh-CN.md">简体中文</a></p>

Crypto comes from Binance, Bybit or OKX public market data, no account needed. US, Hong Kong and China A-share stocks come from Longbridge, with your own OpenAPI keys. It speaks English, Simplified Chinese and Japanese.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="The Candlewick chart window: BTC/USDT one-minute candlesticks updating live, the 24-hour high, low and volume, and a 20-level order book with buy and sell pressure" src="assets/chart-light.png">
</picture>

## Features

- One instrument always in the menu bar or taskbar, and up to 30 in the dropdown
- Crypto: Binance, Bybit or OKX, any spot pair
- Stocks: US (including pre-market, after-hours and overnight), Hong Kong and China A-shares, by code
- A chart that moves with every trade, with the order book and recent trades
- With an exchange's API key, your total holdings and every account's positions
- Turn on AI access and Claude Code, Codex and OpenCode can read your quotes and holdings to think them through with you
- English, 简体中文 or 日本語, following your system or picked in Settings
- Red-up or green-up colors, launch at login, automatic updates

## Install

**macOS**: get `Candlewick_<version>_aarch64.dmg` from the [latest release](https://github.com/gtoxlili/candlewick/releases/latest) and drag Candlewick into Applications. It needs macOS 15 or later on Apple silicon. It lives in the menu bar, with no Dock icon.

**Windows**: run `Candlewick_<version>_x64-setup.exe`. It needs Windows 10 or 11. If SmartScreen stops it, click More info, then Run anyway. The price appears beside the taskbar clock.

## Stocks

1. Copy the App Key, App Secret and Access Token from the user center at [open.longbridge.com](https://open.longbridge.com/).
2. Paste them into Settings → Longbridge.
3. Add stocks by code in the search field, such as AAPL, 700 or 600519.

## Holdings

1. Create an API key on the exchange; read permission is all it needs. OKX also asks for a passphrase.
2. Paste it into Settings → Holdings.
3. The dropdown now leads with Total and its 24h change. Its submenu sums things up: the day's PnL, each account, the largest holdings and open positions; any row opens the holdings window, where the total moves with the market while it is open.
4. Settings → Holdings → Show total in menu bar (taskbar on Windows) puts the total there instead of a price.

## FAQ

**Does it work where the exchange's website is blocked?** Yes. It goes through your system proxy and switches to backup addresses when the main ones don't answer.

**How much does it use?** About 19 MB of memory and under 1% of one CPU core on a Mac.

**Does it collect any data?** No. It only talks to the exchanges and Longbridge you use, and to GitHub for updates (you can turn that off in the settings). Keys stay on your computer, and Candlewick never trades. With AI access on, AI agents on your computer can read its data; other computers and web pages can't.

**Is there a build for Intel Macs?** Not yet.

## Build from source

You need Rust 1.98+, Node.js and pnpm, plus macOS 15 or later on a Mac, or the Visual Studio C++ build tools on Windows.

```sh
pnpm install
pnpm tauri dev     # run in development
pnpm tauri build   # the installer lands in src-tauri/target/release/bundle/
```

[docs/windows.md](docs/windows.md) explains how the Windows version works. Every push to main that changes the app becomes a release, and [docs/release.md](docs/release.md) has the details.

## Project structure

- `src-tauri/src/`: the Rust side, with what the bar shows, the windows and settings
- `src-tauri/src/platform/`: the parts that differ between macOS and Windows
- `src-tauri/src/market/`: prices and holdings, one implementation per source. How to add one: [docs/market-data-providers.md](docs/market-data-providers.md)
- `src-tauri/src/portfolio.rs`: what holdings are worth
- `src-tauri/src/agent/`: AI access, a local read-only API and its skill; see [docs/agent-access.md](docs/agent-access.md)
- `src/settings/`, `src/chart/` and `src/holdings/`: the settings, chart and holdings windows, the chart built on [Liveline](https://github.com/benjitaylor/liveline)
- `src-tauri/locales/` and `src/locales/`: every word in English, Chinese and Japanese, for the app and for its pages; see [docs/i18n.md](docs/i18n.md)
- `patches/liveline@0.0.7.patch`: dragging, zooming and the color setting for Liveline

Built with [Tauri 2](https://tauri.app), Rust, React 19 and Tailwind CSS.

## License

[GPL-3.0](LICENSE). Candlewick is not affiliated with Binance, Bybit, OKX or Longbridge. Prices are for reference only and are not financial advice.
