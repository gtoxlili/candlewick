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

Candlewick shows live crypto and stock prices in the macOS menu bar, or in the Windows taskbar next to the clock. Crypto comes from Binance's public market data, so any spot pair works without an account. US, Hong Kong and China A-share stocks come from Longbridge, with your own OpenAPI keys. Click an entry in the dropdown to open a chart that moves with every trade. The interface is in Simplified Chinese.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="The Candlewick chart window: BTC/USDT one-minute candlesticks updating live, the 24-hour high, low and volume, and a 20-level order book with buy and sell pressure" src="assets/chart-light.png">
</picture>

## Features

- One pinned entry in the menu bar or taskbar, with its name and change if you like, and your whole watchlist in the dropdown
- Up to 30 entries, crypto and stocks side by side: any Binance spot pair, plus stocks by code (AAPL, 700, 600519)
- A line or candlestick chart that updates with every trade, with intervals from 1 second to 1 day for crypto and 1 minute to 1 week for stocks. Drag to scroll back, pinch or scroll to zoom
- The order book with buy and sell pressure, and a live list of recent trades
- US pre-market, after-hours and overnight sessions
- Green-up or red-up colors, and launch at login
- Updates itself: a new version downloads in the background and takes over while the screen is off or locked

## Install

### macOS

Download `Candlewick_<version>_aarch64.dmg` from the [latest release](https://github.com/gtoxlili/candlewick/releases/latest), open it and drag Candlewick into Applications. It needs macOS 15 or later on Apple silicon, and it is signed and notarized by Apple. Candlewick lives in the menu bar and has no Dock icon.

### Windows

Download `Candlewick_<version>_x64-setup.exe` from the [latest release](https://github.com/gtoxlili/candlewick/releases/latest) and run it. It needs Windows 10 or 11, and no administrator rights. The installer isn't code-signed yet, so SmartScreen may stop it the first time. Click More info, then Run anyway.

The price appears in the taskbar beside the clock. Click it for your watchlist and settings.

## Stocks with Longbridge

Stock prices need a Longbridge account with OpenAPI access.

1. Sign in at [open.longbridge.com](https://open.longbridge.com/) and copy the App Key, App Secret and Access Token from the user center.
2. Paste them into Settings → 长桥 (Longbridge) and save. Candlewick logs in and lists your quote access for each market.
3. Add stocks by code in the search field, such as AAPL, 700 or 600519.

Candlewick keeps the keys on your computer only, and renews the access token before it expires.

## FAQ

### Do I need an account?

Not for crypto. Stocks need your own Longbridge keys, which Candlewick only uses to read quotes. It never trades.

### Does it work where binance.com is blocked?

Yes. It goes through your system proxy, and when `binance.com` is unreachable it switches to Binance's public data hosts on `binance.vision`.

### How much memory and CPU does it use?

On a Mac, about 19 MB of memory and under 1% of one CPU core while prices stream. The chart and settings windows exist only while they are open.

### Does it collect any data?

No. It talks to Binance and, once you add keys, to Longbridge. It also asks GitHub for new versions, unless you turn off automatic updates in the settings.

### Is there an English interface, or a build for Intel Macs?

Not yet.

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
- `src-tauri/src/market/`: market data behind one interface, with a provider for Binance and one for Longbridge
- `src/settings/` and `src/chart/`: the settings and chart windows, the chart built on [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`: dragging, zooming and the color setting for Liveline

Built with [Tauri 2](https://tauri.app), Rust, React 19 and Tailwind CSS.

## License

[GPL-3.0](LICENSE). Candlewick is not affiliated with Binance or Longbridge. Prices are for reference only and are not financial advice.
