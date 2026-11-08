---
name: candlewick
description: The user's synced and registered holdings and live market data from Candlewick, a desktop app running on this machine. Holdings across Binance, Bybit and OKX (spot, funding, earn, futures positions with PnL), registered stocks, ETFs, funds and gold held on other platforms, the user's watchlist with live quotes, and candles, recent trades and order books for any crypto pair, any on-chain DEX pool, US, Hong Kong and China A-share stock, or China index, Tonghuashun sector board, exchange-traded fund, CNY fund NAV or Au99.99 gold. Use when the user asks about their portfolio, positions, PnL or exposure, or about a market, or wants to weigh a trade with real numbers (持仓、仓位、盈亏、资产、登记、基金、黄金、ETF、行情、K 线、盘口、链上、池子).
---

# Candlewick

While it runs, Candlewick answers local HTTP requests at `{{url}}`. Each request carries the token stored in `{{token}}`:

```{{shell}}
{{example}}
```

Responses are JSON. Failures are `{"error": "…"}` with a 4xx or 5xx status. Fields are codes and numbers; only messages from the sources (a failure's `error`, an account's `error`) are prose, in the language the user runs Candlewick in.

The API reads market and account data and maintains local records of holdings on other platforms. Registering, editing or deleting a record changes the local portfolio; it places no orders and moves no funds. Quantity and cost are user-maintained; prices come from the selected market source.

## Endpoints

| Request | Returns |
|---|---|
| `GET /v1` | Version, the exchange the quotes come from, the exchanges whose holdings are read, the stock sources set up (`stocks`), registered-holdings support (`manualHoldings: true`), and each source's candle intervals |
| `GET /v1/portfolio` | Synced exchange holdings and valued registered holdings |
| `GET /v1/manual-holdings` | All registered source records |
| `GET /v1/manual-holdings/{id}` | One registered source record |
| `POST /v1/manual-holdings` | Register a quantity; returns the record with HTTP 201 |
| `PUT /v1/manual-holdings/{id}` | Replace a registered record; returns the saved record |
| `DELETE /v1/manual-holdings/{id}` | Delete a local record; HTTP 204 |
| `GET /v1/watchlist` | The user's watchlist with live quotes |
| `GET /v1/search?q=SOL` | `{found: [{id, instrument, metadata, liquidity}], notes}`; `instrument` can be used in a registered holding |
| `GET /v1/candles?id=…&interval=3600&limit=500&end=…` | Candles, oldest first |
| `GET /v1/trades?id=…&limit=100` | Recent trades, oldest first |
| `GET /v1/market?id=…` | Day statistics and up to 20 order book levels per side |

An instrument id is `source:symbol`: `binance:BTCUSDT`, `bybit:BTCUSDT`, `okx:BTC-USDT`, `longbridge:AAPL.US`, `longbridge:700.HK`, `longbridge:600519.SH`, `ths:600519.SH`, and for an on-chain pool `dexscreener:<chain>/<pool address>` as in DEX Screener's URLs, e.g. `dexscreener:solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx`. Candles, trades and market take any symbol the source lists, whether or not it is in the watchlist. Stocks come from two sources, each needing the user's credentials in Candlewick: `longbridge` (US, Hong Kong and China A-shares) and `ths`, Tonghuashun (China only: A-shares such as `ths:600519.SH`, the exchanges' indexes such as `ths:000001.SH`, Tonghuashun's sector boards such as `ths:885525.TI`, ETFs such as `ths:510300.SH`, and LOFs). Both write a China share the same way, so `600519.SH` can be in the watchlist from either. `/v1/search` finds `ths` codes by Chinese name as well as by code. Pools are priced in USD whatever their quote token; their `quote` is `USD`, `base` the token's symbol, and `venue` names the chain and DEX. `/v1/search` finds pools by token symbol, name or address, with each pool's `liquidity` in USD to tell the pools of one token apart.

Eastmoney is also available through the shared search, watchlist and market endpoints, independently of registered holdings. Eastmoney needs no credentials. `eastmoney:150.<six-digit code>` supplies the latest published CNY unit NAV and, when available, the previous published NAV, polled every 15 minutes. This is daily fund data, not an intraday estimate. `priceUpdated` encodes the NAV date at Shanghai midnight; the date uses UTC+8. Income-per-10,000-share money-market series and foreign-currency fund shares are unsupported. `eastmoney:118.AU9999` is Shanghai Gold Exchange gold in CNY per gram, polled every 15 seconds, with the source quote timestamp. Discovery uses source metadata; NAV and price availability are checked by the quote feed. A missing reference NAV or previous close leaves the price usable, with null changes. Eastmoney history and trades are empty; market supplies snapshot statistics. Stock sources retain their credential requirements.

Search results, watchlist entries, market responses and valued registered holdings include `metadata`: `{assetId, quantityUnit, priceKind, usdConversion}`. `quantityUnit` is `tokens`, `shares`, `fundUnits`, `grams` or `units`. `priceKind` is `{kind: "market"}` or `{kind: "nav", utcOffset: seconds east of UTC}`; the latter supplies the timezone for the NAV's calendar date. `usdConversion` is an additional quoted instrument used to convert a crypto quote currency, or null. These properties belong to the market source and apply throughout the app.

## Registered holdings

A platform describes where the user holds an asset. An instrument describes the market data used to value it. These are independent: an ETF held at a broker can use Tonghuashun quotes; a fund held in Alipay can use Eastmoney NAVs. A record does not require a synced account or a watchlist entry.

`POST` and `PUT` accept the complete body below. `instrument` is the object returned by search, including its provider, symbol, base, quote and available name/decimals/venue. `PUT` replaces the whole record. IDs and timestamps are assigned by Candlewick. Missing records return 404, invalid bodies return 400, storage failures return 500. Writes persist before appearing in the portfolio. At most 500 records are supported.

```json
{
  "platform": "Alipay",
  "instrument": {
    "provider": "eastmoney",
    "symbol": "150.000216",
    "base": "000216",
    "quote": "CNY",
    "name": "华安黄金ETF联接A"
  },
  "amount": 1000,
  "cost": 2.8
}
```

`amount` is a finite positive quantity. `cost` is the optional average cost per unit in `instrument.quote`, or null. Stock/ETF quantities are shares or fund units; ordinary funds use fund shares; `eastmoney:118.AU9999` uses grams. There is no price input. Gold funds and gram-based gold products have different units and instruments. Registering the same holding as a synced account counts it twice.

`GET /v1/manual-holdings` returns `[{id, platform, instrument, amount, cost, updated}]`. `updated` is the last quantity/cost edit in epoch ms, independent of market data freshness. Records persist locally across restarts; exchange account refreshes leave them intact.

## Portfolio

`{valuation, accounts, total, change, assets, positions, manualHoldings, manualError, partial}`. Monetary values use `valuation.currency`, one of `CNY`, `USD` or `JPY` selected in Candlewick.

- `valuation`: `{currency, rate, date, error}`. `rate` is the number of currency units per USD; `date` is the daily reference rate's date. USD is the base valuation unit, with `rate: 1` and null date/error. The app values USD and USDT 1:1 when using exchange USDT pairs. CNY and JPY use daily exchange rates. `currency` is the actual unit of the returned values. If no foreign exchange rate is available, values use USD at rate 1, including the wallet and asset breakdowns. `error` reports a failed rate refresh; cached rates remain usable with their date.

- `accounts`: one per exchange with a key: `{exchange, total, change, wallets: [{wallet, value}], updated, error}`. `wallets` says where the money sits: `spot` (Binance spot), `trading` (Bybit's unified account, OKX's trading account: spot and derivatives together), `funding`, `earn`, `usdFutures` and `coinFutures` (Binance's USDⓈ-M and COIN-M futures wallets). `total` and `change` are null until the account has been valued once.
- `assets` merges the same identity into one line, most valuable first: `{id, asset, amount, price, value, partial, change, changePct, stable, held, cost, pnl}`, `held` contains either `{exchange, wallet, amount}` or `{manualId, platform, amount}`. Identity is `crypto:<base>` for exchange coins, `stock:<symbol>` for stocks/ETFs and the source instrument ID for other assets. Each synced asset is valued at the exchange's own price for its USDT pair; without one, at the exchange's own USD figure where it gives one, otherwise `value` is null and the asset isn't in `total`. `partial` marks an asset whose value covers only its priced quantities; its combined price is then null. `stable` marks dollar stablecoins (cash).
- Trading and futures accounts count at equity: the balance plus the unrealized PnL of their positions. `positions` are therefore already inside `total`; their `pnl` is part of it, not an addition.
- `manualHoldings` contains the source records plus `{metadata, price, reference, priceUpdated, received, value, change, pnl, stale, error}`. `price`, `reference` and the record's `cost` retain `instrument.quote`; `value`, `change` and `pnl` use `valuation.currency`. `priceUpdated` is the source timestamp where available, and `received` is when Candlewick received the quote, both epoch ms. `stale` reports a source outage or pause; cached prices remain visible. Missing quotes or required FX rates produce null values and an error; they are excluded from the total, never treated as zero or as USD.
- `partial` marks a total missing one or more valuations, including an unreadable registered-holdings file (`manualError`). `total` is null until at least one holding is valued. Registered prices in CNY, HKD and JPY convert through the daily USD reference rates; crypto pairs quoted in another coin use that coin's USDT feed.
- `change` is the price move applied to the quantity held now. Crypto uses the 24h reference, stocks/ETFs the source's previous close, and funds the previous published NAV. The combined figure mixes those reference periods and excludes FX movements and changes in quantities. It is null when a required valuation/reference is unavailable. Synced accounts retain their own 24h moves. An asset containing an unknown price move has null `change` and `changePct`.
- `asset.cost` is the quantity-weighted average cost where all merged holdings have a cost, otherwise null. `asset.pnl` combines available unrealized cost PnL only when all merged parts report it. Registered costs use the instrument's quote currency and are converted for the asset row; synced cost data comes from OKX.
- A position has `exchange`, `symbol`, `kind`, `long`, `size` in `sizeUnit` (a base asset, or null for a number of contracts), `entry`, `mark`, `liquidation`, `leverage`, `isolated`, `exposure` in USD at the mark (negative when short), and `pnl` in `pnlAsset`. `pnlValue`, `exposureValue` and `change` use `valuation.currency`; each is null if its source cannot be valued. Contract prices (`entry`, `mark`, `liquidation`), `size`, native `pnl` retain the contract's units; `exposure` retains the dollar valuation convention. Largest valued PnL, either way, first. `kind` is what it settles in and whether it expires: `usdtPerpetual`, `usdtFutures`, `usdcPerpetual`, `usdcFutures`, `coinPerpetual`, `coinFutures` (coin-margined, inverse), `option`, `margin` (OKX spot margin) or `other`.
- `updated` (epoch ms) is when the account's balances were last read. A request first refreshes accounts older than 15 seconds, which takes a few seconds. `error` explains a failed read; the account's numbers are then those from `updated`.

## Watchlist

`[{id, metadata, symbol, base, quote, name, venue, pinned, last, reference, priceUpdated, received, changePct, session}]`

- `reference` is the price 24 hours ago for crypto and pools, the last regular close for stocks, and the prior published NAV for funds; `changePct` is measured against it.
- `session` is `pre`, `post` or `overnight` for US stocks outside regular hours, null otherwise.
- `priceUpdated` is the optional source timestamp and `received` the quote receipt time, in epoch ms. `last` is null until the first quote arrives. `pinned` marks the one shown in the menu bar.

## Candles

`[{time, open, high, low, close}]`; `time` is the epoch second the candle opened, and the last candle is still forming.

- `interval` is in seconds and must be one of the source's intervals listed by `GET /v1`.
- One call returns at most `limit` candles, and fewer where the source pages smaller (OKX: 300; the others: 1000). `end` (epoch seconds) asks for candles that opened before it, so paging back means passing the oldest `time` received. An empty list means nothing older exists.
- Crypto and pool days open at 00:00 UTC; stock candles follow the exchange's sessions and days.
- `ths` has daily and weekly candles only, weeks opening on Monday. Its indexes and ETFs reach back five years, and LOFs have no candles (an empty list). Its shares are forward-adjusted the way Tonghuashun does it, by taking dividends off the price, so early prices run low; days whose adjusted price would be zero or below are left out.
- Pool candles come from GeckoTerminal, which allows only a few requests a minute; a request made while it is rate limiting fails at once with a message saying how long to wait.

## Trades

`[{id, price, qty, time, sell, extended}]`: `time` is epoch milliseconds, `sell` means the taker sold, `extended` marks a US stock trade outside regular hours. A pool's trades are its last 300 swaps of the past 24 hours, `qty` in the token, `sell` meaning the token was sold. `ths` keeps no trades: its list is always empty.

## Market

`{metadata, stats, book}`; `stats` and `book` are null if they did not arrive in time.

- `stats`: `{last, high, low, volume, turnover, change, changePct, onchain}`; volume in the base asset, turnover in the quote; `high`, `low` and `volume` are null where the source gives none. `change` and `changePct` are null when the reference price is unavailable, while `last` remains usable. Crypto and pool stats cover the last 24 hours, stocks the current day. For a pool, `onchain` is `{liquidity, marketCap, fdv, buys, sells}`: USD in the pool, the token's market cap and fully diluted valuation (either may be null), and the day's swap counts; elsewhere it is null.
- `book`: `{bids, asks}`, best level first, each `{price, qty}`; null for pools and `ths`, which have no order book.
- `ths` stats come from a snapshot that Candlewick reads every few seconds while Shanghai trades; `volume` is in shares, fund units for funds and the members' shares for an index.
- It opens a live connection to the source and waits for its first data, which takes up to a few seconds.
