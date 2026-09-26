<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Candlewick app icon: a glowing live price line ending in a pulsing dot">
</p>

<h1 align="center">Candlewick</h1>

<p align="center">Live crypto and stock prices in the macOS menu bar, with a real-time chart, order book and trades.</p>

<p align="center">
  <a href="https://github.com/gtoxlili/candlewick/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/gtoxlili/candlewick"></a>
  <img alt="macOS 15 or later on Apple silicon" src="https://img.shields.io/badge/macOS-15%2B%20%C2%B7%20Apple%20silicon-black">
  <a href="LICENSE"><img alt="License: GPL-3.0" src="https://img.shields.io/github/license/gtoxlili/candlewick"></a>
</p>

<p align="center"><a href="README.zh-CN.md">简体中文</a></p>

Candlewick is a free, open-source macOS menu bar app that shows live prices of cryptocurrencies and stocks. Crypto comes from Binance's public market data: any spot pair, with no account or key. US, Hong Kong and China A-share stocks come from Longbridge, with your own OpenAPI credentials. Pin one entry to the menu bar, and click any in the dropdown to open a chart that moves with every trade. The interface is in Simplified Chinese.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/chart-dark.png">
  <img alt="The Candlewick chart window: BTC/USDT one-minute candlesticks updating live, the 24-hour high, low and volume, and a 20-level order book with buy and sell pressure" src="assets/chart-light.png">
</picture>

## Features

- The price in the menu bar: the pinned entry shows its last price, optionally with its name and change stacked in two compact rows, and the dropdown lists your whole watchlist
- Crypto and stocks side by side, up to 30 entries: any Binance spot pair, and US, Hong Kong and China A-share stocks by code (AAPL, 700, 600519)
- US stocks follow pre-market, post-market and overnight trading, with the session marked in the dropdown
- A live chart: line or candlesticks, from 1 second to 1 day for crypto and 1 minute to 1 week for stocks, updated by every trade and animated by [Liveline](https://github.com/benjitaylor/liveline); drag to scroll back through history, scroll or pinch to zoom
- The order book with buy and sell pressure (20 levels for crypto, grouped by the price step you pick; for stocks, as many as your Longbridge quote access gives), and a live list of recent trades
- Green-up or red-up colors, and launch at login
- Pauses while the Mac or its display sleeps, and reconnects on its own

## Install

Download `Candlewick_<version>_aarch64.dmg` from the [latest release](https://github.com/gtoxlili/candlewick/releases/latest), open it and drag Candlewick into Applications. It needs macOS 15 or later on a Mac with Apple silicon. The app is signed with a Developer ID and notarized by Apple, so macOS opens it normally.

Candlewick lives in the menu bar and has no Dock icon; open its settings from the dropdown menu.

## Stocks with Longbridge

Stock prices need a Longbridge account with OpenAPI access:

1. Sign in at [open.longbridge.com](https://open.longbridge.com/) and copy the App Key, App Secret and Access Token from the user center.
2. Paste them into Settings → 长桥 (Longbridge) and save. Candlewick logs in and lists your quote access for each market.
3. Add stocks by code in the search field, such as AAPL, 700 or 600519.

The credentials stay on your Mac, in `~/Library/Application Support/com.influo.candlewick/credentials.json`, readable only by your user account. The access token is renewed automatically before it expires.

## FAQ

### Do I need an account or an API key?

Not for crypto: Candlewick reads Binance's public spot market data, over WebSocket and REST, and never trades. Stocks need your own Longbridge OpenAPI credentials (see above), which Candlewick only uses to read quotes.

### What can I track?

Any Binance spot trading pair, such as BTC/USDT, ETH/USDT or ETH/BTC, and with Longbridge, US, Hong Kong and China A-share stocks and ETFs. Up to 30 entries at a time.

### Does it work where binance.com is blocked?

It follows the macOS system proxy settings (HTTPS and SOCKS). When `binance.com` cannot be reached, it switches to Binance's public market data hosts on `binance.vision`. Longbridge is reached through its mainland China hosts where those answer.

### How much memory and CPU does it use?

Sitting in the menu bar with prices streaming, about 19 MB of memory and under 1% of one CPU core. The chart and settings windows exist only while they are open.

### Does it collect any data?

No. There is no telemetry; the app only talks to Binance's market data endpoints and, once you add credentials, to Longbridge's OpenAPI. Your settings and credentials stay on your Mac.

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

- `src-tauri/src/`: the Rust side: the menu bar item, windows and settings
- `src-tauri/src/market/`: market data behind one interface, with a provider for Binance and one for Longbridge
- `src/settings/`: the settings window
- `src/chart/`: the chart window, built on [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`: adds dragging and zooming to Liveline, makes it follow the color setting, and lowers its frame rate while the window is in the background

Built with [Tauri 2](https://tauri.app), Rust, React 19 and Tailwind CSS.

## License

[GPL-3.0](LICENSE). Candlewick is not affiliated with Binance or Longbridge. Prices are for reference only and are not financial advice.
