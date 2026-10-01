---
name: candlewick
description: The user's crypto holdings and live market data from Candlewick, a desktop app running on this machine. Holdings across Binance, Bybit and OKX (spot, funding, earn, futures positions with PnL), the user's watchlist with live quotes, and candles, recent trades and order books for any crypto pair or US, Hong Kong and China A-share stock. Use when the user asks about their portfolio, positions, PnL or exposure, or about a market, or wants to weigh a trade with real numbers (持仓、仓位、盈亏、资产、行情、K 线、盘口).
---

# Candlewick

While it runs, Candlewick answers read-only HTTP requests at `{{url}}`. Each request carries the token stored in `{{token}}`:

```{{shell}}
{{example}}
```

Responses are JSON. Failures are `{"error": "…"}` with a 4xx or 5xx status.

The API only reads: it places no orders, moves no funds and changes no settings. Trades happen in the exchanges' own apps.

## Endpoints

| Request | Returns |
|---|---|
| `GET /v1` | Version, the exchange the quotes come from, the exchanges whose holdings are read, whether stocks are set up, and each source's candle intervals |
| `GET /v1/portfolio` | Holdings on every exchange with an API key |
| `GET /v1/watchlist` | The user's watchlist with live quotes |
| `GET /v1/search?q=SOL` | Instruments matching a name or code, as ids |
| `GET /v1/candles?id=…&interval=3600&limit=500&end=…` | Candles, oldest first |
| `GET /v1/trades?id=…&limit=100` | Recent trades, oldest first |
| `GET /v1/market?id=…` | Day statistics and up to 20 order book levels per side |

An instrument id is `source:symbol`: `binance:BTCUSDT`, `bybit:BTCUSDT`, `okx:BTC-USDT`, `longbridge:AAPL.US`, `longbridge:700.HK`, `longbridge:600519.SH`. Candles, trades and market take any symbol the source lists, whether or not it is in the watchlist. Stocks (`longbridge`) need the user's Longbridge credentials in Candlewick.

## Portfolio

`{accounts, total, change, assets, positions}`, amounts in USDT.

- `accounts`: one per exchange with a key: `{exchange, name, total, change, wallets: [{wallet, label, value}], updated, error}`. `wallets` says where the money sits: 现货 (Binance spot), 交易账户 (Bybit's unified account, OKX's trading account: spot and derivatives together), 资金 (funding), 理财 (earn), U 本位合约 and 币本位合约 (Binance's futures wallets). `total` and `change` are null until the account has been read once.
- `assets` merges the same asset across exchanges into one line, most valuable first: `{asset, amount, price, value, change, changePct, stable, held: [{exchange, wallet, amount}], cost, pnl}`. Each is valued at the exchange's own price for its USDT pair; without one, at the exchange's own USD figure where it gives one, otherwise `value` is null and the asset isn't in `total`. `stable` marks dollar stablecoins (cash).
- Trading and futures accounts count at equity: the balance plus the unrealized PnL of their positions. `positions` are therefore already inside `total`; their `pnl` is part of it, not an addition.
- `change` is what the last 24 hours' price moves made of what is held now: each asset's amount times its 24h price change (`asset.change`), plus each position's dollar exposure times its contract's 24h move (`position.change`). Deposits, withdrawals and trades during the day are not in it. `asset.changePct` is the asset's own 24h price change.
- `asset.cost` and `asset.pnl` come from OKX only: the spot holding's average cost and unrealized PnL, in USD. Elsewhere they are null.
- A position has `exchange`, `symbol`, `kind` (U 本位永续, 币本位交割, 期权…), `long`, `size` in `sizeUnit` (a base asset, or 张 for contracts), `entry`, `mark`, `liquidation`, `leverage`, `isolated`, `exposure` (USD at the mark, negative when short), and `pnl` in `pnlAsset` with `pnlUsd`. Largest PnL, either way, first.
- `updated` (epoch ms) is when the account's balances were last read. A request first refreshes accounts older than 15 seconds, which takes a few seconds. `error` explains a failed read; the account's numbers are then those from `updated`.

## Watchlist

`[{id, symbol, base, quote, name, pinned, last, reference, changePct, session}]`

- `reference` is the price 24 hours ago for crypto and the last regular close for stocks; `changePct` is measured against it.
- `session` is 盘前, 盘后 or 夜盘 for US stocks outside regular hours.
- `last` is null until the first quote arrives. `pinned` marks the one shown in the menu bar.

## Candles

`[{time, open, high, low, close}]`; `time` is the epoch second the candle opened, and the last candle is still forming.

- `interval` is in seconds and must be one of the source's intervals listed by `GET /v1`.
- One call returns at most `limit` candles, and fewer where the source pages smaller (OKX: 300; the others: 1000). `end` (epoch seconds) asks for candles that opened before it, so paging back means passing the oldest `time` received. An empty list means nothing older exists.
- Crypto days open at 00:00 UTC; stock candles follow the exchange's sessions and days.

## Trades

`[{id, price, qty, time, sell, extended}]`: `time` is epoch milliseconds, `sell` means the taker sold, `extended` marks a US stock trade outside regular hours.

## Market

`{stats, book}`, each null if it didn't arrive in time.

- `stats`: `{last, high, low, volume, turnover, change, changePct}`; volume in the base asset, turnover in the quote. Crypto stats cover the last 24 hours, stocks the current day.
- `book`: `{bids, asks}`, best level first, each `{price, qty}`.
- It opens a live connection to the source and waits for its first data, which takes up to a few seconds.
