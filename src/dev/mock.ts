// Runs the pages in a plain browser (`pnpm dev`, then open /holdings.html
// etc. in Chrome) with the app's IPC answered from fixtures, so a page can be
// iterated on without the app. Only the development build imports this, and
// only when Tauri's IPC is absent (see each page's main.tsx).

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { Portfolio, Settings } from "@/lib/api";
import { LOCALES, type Locale } from "@/lib/i18n";

const settings: Settings = {
  exchange: "binance",
  watchlist: [
    { provider: "binance", symbol: "BTCUSDT", base: "BTC", quote: "USDT", decimals: 2, pinned: true },
    { provider: "binance", symbol: "ETHUSDT", base: "ETH", quote: "USDT", decimals: 2, pinned: false },
  ],
  showSymbol: true,
  showChange: false,
  holdingsInBar: false,
  concealHoldings: false,
  colorScheme: "greenUp",
  autoUpdate: true,
  agentAccess: false,
  language: "system",
};

const now = Date.now();

/** Two exchanges, a few assets on both, two futures positions. */
const portfolio: Portfolio = {
  accounts: [
    {
      exchange: "binance",
      total: 78_210.1,
      change: 712.3,
      wallets: [
        { wallet: "spot", value: 58_120.4 },
        { wallet: "earn", value: 12_000 },
        { wallet: "usdFutures", value: 8_089.7 },
      ],
      updated: now - 4_000,
      error: null,
    },
    {
      exchange: "okx",
      total: 50_220.42,
      change: 860,
      wallets: [
        { wallet: "trading", value: 48_020.42 },
        { wallet: "funding", value: 2_200 },
      ],
      updated: now - 9_000,
      error: null,
    },
  ],
  total: 128_430.52,
  change: 1_572.3,
  assets: [
    {
      asset: "BTC",
      amount: 0.7612,
      price: 84_100.77,
      value: 64_017.5,
      change: 1_320.2,
      changePct: 2.1,
      stable: false,
      held: [
        { exchange: "binance", wallet: "spot", amount: 0.5 },
        { exchange: "okx", wallet: "trading", amount: 0.2612 },
      ],
      cost: 61_200,
      pnl: 5_980.2,
    },
    {
      asset: "ETH",
      amount: 6.4,
      price: 3_412.1,
      value: 21_837.4,
      change: -210.6,
      changePct: -0.95,
      stable: false,
      held: [
        { exchange: "binance", wallet: "spot", amount: 4 },
        { exchange: "binance", wallet: "earn", amount: 2.4 },
      ],
      cost: null,
      pnl: null,
    },
    {
      asset: "USDT",
      amount: 30_890.2,
      price: 1,
      value: 30_890.2,
      change: 0,
      changePct: 0,
      stable: true,
      held: [
        { exchange: "okx", wallet: "trading", amount: 20_010.2 },
        { exchange: "binance", wallet: "usdFutures", amount: 8_680 },
        { exchange: "okx", wallet: "funding", amount: 2_200 },
      ],
      cost: null,
      pnl: null,
    },
    {
      asset: "SOL",
      amount: 42.5,
      price: 178.4,
      value: 7_582,
      change: 310.4,
      changePct: 4.27,
      stable: false,
      held: [{ exchange: "okx", wallet: "trading", amount: 42.5 }],
      cost: 150.1,
      pnl: 1_202.7,
    },
    {
      asset: "USDC",
      amount: 2_000,
      price: 1,
      value: 2_000,
      change: 0,
      changePct: 0,
      stable: true,
      held: [{ exchange: "binance", wallet: "spot", amount: 2_000 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "BNB",
      amount: 1.8,
      price: 612.3,
      value: 1_102.1,
      change: 12.4,
      changePct: 1.14,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 1.8 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "PEPE",
      amount: 52_000_000,
      price: 0.00001234,
      value: 641.7,
      change: 140.1,
      changePct: 27.9,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 52_000_000 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "ARB",
      amount: 310,
      price: 1.17,
      value: 362.7,
      change: -8.2,
      changePct: -2.2,
      stable: false,
      held: [{ exchange: "okx", wallet: "trading", amount: 310 }],
      cost: 1.4,
      pnl: -71.3,
    },
    {
      asset: "DOGE",
      amount: 12,
      price: 0.21,
      value: 2.52,
      change: 0.04,
      changePct: 1.6,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 12 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "ATOM",
      amount: 0.03,
      price: 8.1,
      value: 0.24,
      change: 0,
      changePct: 0.3,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 0.03 }],
      cost: null,
      pnl: null,
    },
    {
      asset: "NOPAIR",
      amount: 3,
      price: null,
      value: null,
      change: null,
      changePct: null,
      stable: false,
      held: [{ exchange: "binance", wallet: "spot", amount: 3 }],
      cost: null,
      pnl: null,
    },
  ],
  positions: [
    {
      exchange: "binance",
      symbol: "BTCUSDT",
      kind: "usdtPerpetual",
      long: true,
      size: 0.4,
      sizeUnit: "BTC",
      entry: 81_240,
      mark: 84_100.77,
      liquidation: 62_310,
      leverage: 5,
      isolated: false,
      pnl: 1_144.3,
      pnlAsset: "USDT",
      pnlUsd: 1_144.3,
      exposure: 33_640.3,
      change: 690.1,
    },
    {
      exchange: "okx",
      symbol: "ETH-USDT-SWAP",
      kind: "usdtPerpetual",
      long: false,
      size: 20,
      sizeUnit: null,
      entry: 3_380,
      mark: 3_412.1,
      liquidation: 3_610,
      leverage: 10,
      isolated: true,
      pnl: -64.2,
      pnlAsset: "USDT",
      pnlUsd: -64.2,
      exposure: -6_824.2,
      change: 65,
    },
  ],
};

/** A portfolio before any account has answered. */
const loading: Portfolio = {
  accounts: [{ exchange: "binance", total: null, change: null, wallets: [], updated: null, error: null }],
  total: null,
  change: null,
  assets: [],
  positions: [],
};

/**
 * Answers the app's commands for `label`'s page. `?state=empty|loading` picks
 * a portfolio; `?lang=en|zh-CN|ja` the language the app speaks (default: the
 * first of the browser's that it has).
 */
export function installMocks(label: string): void {
  mockWindows(label);
  const params = new URLSearchParams(location.search);
  const state = params.get("state");
  const locales = Object.keys(LOCALES) as Locale[];
  const speaks = (tag: string) => locales.find((l) => l === tag) ?? locales.find((l) => l.split("-")[0] === tag.split("-")[0]);
  const locale = [params.get("lang") ?? "", ...navigator.languages].map(speaks).find((l) => l !== undefined) ?? "en";
  mockIPC((cmd) => {
    switch (cmd) {
      case "get_settings":
        return settings;
      case "get_locale":
        return locale;
      case "get_portfolio":
        return state === "empty" ? { ...loading, accounts: [] } : state === "loading" ? loading : portfolio;
      case "plugin:event|listen":
        return 1;
      default:
        return null;
    }
  });
}
