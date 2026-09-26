<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Coin Tray app icon: a glowing live price line ending in a pulsing dot">
</p>

<h1 align="center">Coin Tray</h1>

<p align="center">Live Binance crypto prices in the macOS menu bar, with a real-time chart, order book and trades.</p>

<p align="center">
  <a href="https://github.com/gtoxlili/coin-tray/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/gtoxlili/coin-tray"></a>
  <img alt="macOS 15 or later on Apple silicon" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <a href="LICENSE"><img alt="License: GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/coin-tray"></a>
</p>

<p align="center"><a href="README.zh-CN.md">简体中文</a></p>

Coin Tray is a free, open-source macOS menu bar app that shows live prices of Bitcoin, Ethereum and any other Binance spot pair. Pin the pairs you care about to the menu bar, and click one to open a chart that moves with every trade. It reads Binance's public market data, so there is no account, API key or sign-up. The interface is in Simplified Chinese.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="The Coin Tray chart window: BTC/USDT one-minute candlesticks updating live, the 24-hour high, low and volume, and a 20-level order book with buy and sell pressure" src="assets/chart-light.png">
</picture>

## Features

- The price in the menu bar: the pinned pair shows its last price, optionally with its symbol and 24-hour change stacked in two compact rows, and the dropdown lists every pair you track
- A live chart: line or candlesticks at intervals from 1 second to 1 day, updated by every trade and animated by [Liveline](https://github.com/benjitaylor/liveline); drag to scroll back through history, scroll or pinch to zoom
- The top 20 levels of the order book with buy and sell pressure, and a live list of recent trades
- Any Binance spot pair, up to 30: search, pin and reorder them
- Green-up or red-up colors, and launch at login
- Pauses while the Mac or its display sleeps, and reconnects on its own

## Install

Download `Coin-Tray_<version>_aarch64.dmg` from the [latest release](https://github.com/gtoxlili/coin-tray/releases/latest), open it and drag Coin Tray into Applications. It needs macOS 15 or later on a Mac with Apple silicon. The app is signed with a Developer ID and notarized by Apple, so macOS opens it normally.

Coin Tray lives in the menu bar and has no Dock icon; open its settings from the dropdown menu.

## FAQ

### Do I need a Binance account or an API key?

No. Coin Tray only reads Binance's public spot market data, over WebSocket and REST. It never trades and never asks for a key.

### Which coins can I track?

Any Binance spot trading pair, such as BTC/USDT, ETH/USDT, SOL/USDT or ETH/BTC, up to 30 at a time.

### Does it work where binance.com is blocked?

It follows the macOS system proxy settings (HTTPS and SOCKS). When `binance.com` cannot be reached, it switches to Binance's public market data hosts on `binance.vision`.

### How much memory and CPU does it use?

Sitting in the menu bar with prices streaming, about 19 MB of memory and under 1% of one CPU core. The chart and settings windows exist only while they are open.

### Does it collect any data?

No. There is no telemetry and no account; the app only talks to Binance's market data endpoints, and your settings stay on your Mac.

### Is there an English interface, or a build for Intel Macs?

Not yet. The interface is in Simplified Chinese, and releases are built for Apple silicon.

## Build from source

Requires macOS 15 or later, Rust 1.98+, Node.js and pnpm.

```sh
pnpm install
pnpm tauri dev     # run in development
pnpm tauri build   # the .app and .dmg land in src-tauri/target/release/bundle/
```

## Project structure

- `src-tauri/src/`: the Rust side: the menu bar item, the market data connection, windows and settings
- `src/settings/`: the settings window
- `src/chart/`: the chart window, built on [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`: adds dragging and zooming to Liveline, makes it follow the color setting, and lowers its frame rate while the window is in the background

Built with [Tauri 2](https://tauri.app), Rust, React 19 and Tailwind CSS.

## License

[GPL-3.0](LICENSE). Coin Tray is not affiliated with Binance. Prices are for reference only and are not financial advice.
