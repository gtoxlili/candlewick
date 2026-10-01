// Number and time formatting shared by the windows; the rules match the tray's
// (src-tauri/src/format.rs) so a price reads the same everywhere.

const fixed = new Map<number, Intl.NumberFormat>();

/** Comma-grouped with a fixed number of decimals: 84049.95 → "84,049.95". */
export function fmtPrice(value: number, decimals: number): string {
  let format = fixed.get(decimals);
  if (!format) {
    format = new Intl.NumberFormat("en-US", {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals,
    });
    fixed.set(decimals, format);
  }
  return format.format(value);
}

/** The pair's tick precision when known, otherwise about six significant digits. */
export function priceDecimals(price: number, tickDecimals: number | null): number {
  if (tickDecimals !== null) return tickDecimals;
  if (!(price > 0)) return 2;
  const intDigits = Math.floor(Math.log10(price)) + 1;
  return Math.min(Math.max(6 - intDigits, 2), 10);
}

const compact = new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 2 });

/** Chinese compact units: 12345 → "1.23万", 1034567890 → "10.35亿". */
export function fmtCompact(value: number): string {
  return compact.format(value);
}

/** Order-book and trade sizes: compact when huge, fewer decimals as they grow. */
export function fmtQty(value: number): string {
  if (value >= 100_000) return fmtCompact(value);
  const decimals = value >= 1000 ? 2 : value >= 1 ? 4 : 5;
  return fmtPrice(value, decimals);
}

const amounts = new Map<number, Intl.NumberFormat>();

/** Holdings: up to eight decimals for small amounts, fewer as they grow, none trailing. */
export function fmtAmount(value: number): string {
  const decimals = value >= 1000 ? 2 : value >= 1 ? 4 : 8;
  let format = amounts.get(decimals);
  if (!format) {
    format = new Intl.NumberFormat("en-US", { maximumFractionDigits: decimals });
    amounts.set(decimals, format);
  }
  return format.format(value);
}

/** "+12.34", "−0.41", "0.00": a signed amount with fixed decimals, signed as it rounds. */
export function fmtSigned(value: number, decimals: number): string {
  const text = fmtPrice(Math.abs(value), decimals);
  const sign = Math.sign(Number(value.toFixed(decimals)));
  return sign > 0 ? `+${text}` : sign < 0 ? `−${text}` : text;
}

/** "+1.23%", "−0.41%" (U+2212, as wide as "+"), "0.00%". */
export function fmtPct(pct: number): string {
  const hundredths = Math.round(pct * 100);
  if (hundredths > 0) return `+${pct.toFixed(2)}%`;
  if (hundredths < 0) return `−${(-pct).toFixed(2)}%`;
  return "0.00%";
}

/** Sign of a change after rounding to what is displayed: 1, -1 or 0. */
export function direction(pct: number): 1 | -1 | 0 {
  const hundredths = Math.round(pct * 100);
  return hundredths > 0 ? 1 : hundredths < 0 ? -1 : 0;
}

const clock = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

/** Epoch milliseconds → "14:03:27" in local time. */
export function fmtClock(ms: number): string {
  return clock.format(ms);
}
