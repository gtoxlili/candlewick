// Number and time formatting shared by the windows; the rules match the tray's
// (src-tauri/src/format.rs) so a price reads the same everywhere. Prices,
// amounts and the clock read the same in every language, as trading screens
// write them; what is said in words (compact units, durations, dates) takes
// the page's locale.

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

/** One formatter per key (a locale, and what else tells them apart), made on first use. */
function cached<K extends string[], T>(make: (...key: K) => T): (...key: K) => T {
  const made = new Map<string, T>();
  return (...key) => {
    const id = key.join(" ");
    let found = made.get(id);
    if (found === undefined) {
      found = make(...key);
      made.set(id, found);
    }
    return found;
  };
}

const compact = cached(
  (locale: string) => new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 2 }),
);

/** Compact units as the language counts: 12345 → "1.23万" (zh, ja), "12.35K" (en). */
export function fmtCompact(value: number, locale: string): string {
  return compact(locale).format(value);
}

/** Order-book and trade sizes: compact when huge, fewer decimals as they grow. */
export function fmtQty(value: number, locale: string): string {
  if (value >= 100_000) return fmtCompact(value, locale);
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

const clock = new Intl.DateTimeFormat("en-GB", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});

/** Epoch milliseconds → "14:03:27" in local time, a 24-hour clock in every language. */
export function fmtClock(ms: number): string {
  return clock.format(ms);
}

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86_400;

const units = cached(
  (locale: string, unit: string) => new Intl.NumberFormat(locale, { style: "unit", unit, unitDisplay: "long" }),
);

/** A stretch of time in words, rounded to its largest unit: "45 seconds", "15分钟", "1.5 時間". */
export function fmtDuration(secs: number, locale: string): string {
  const tenths = (n: number) => (n < 10 ? Math.round(n * 10) / 10 : Math.round(n));
  const [value, unit] =
    secs < MINUTE
      ? [Math.round(secs), "second"]
      : secs < HOUR
        ? [Math.round(secs / MINUTE), "minute"]
        : secs < 2 * DAY
          ? [tenths(secs / HOUR), "hour"]
          : [tenths(secs / DAY), "day"];
  return units(locale, unit).format(value);
}

const CLOCK: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit", hourCycle: "h23" };
const DATE: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };

/** How a stretch reads, by how long it is and where it falls. */
const RANGES = {
  clock: CLOCK,
  clockSeconds: { ...CLOCK, second: "2-digit" },
  datedClock: { ...DATE, ...CLOCK },
  datedClockSeconds: { ...DATE, ...CLOCK, second: "2-digit" },
  dates: DATE,
  datesWithYears: { ...DATE, year: "numeric" },
} satisfies Record<string, Intl.DateTimeFormatOptions>;

const ranges = cached(
  (locale: string, shape: keyof typeof RANGES) => new Intl.DateTimeFormat(locale, RANGES[shape]),
);

/**
 * A stretch between two epoch seconds, the language's way: clock times
 * within a day or two (seconds too below ten minutes; dates unless all of
 * it is today), dates beyond (years unless this one). "14:00–15:30",
 * "5月3日 22:00～5月4日 01:30", "May 3 – 9".
 */
export function fmtRange(from: number, to: number, locale: string): string {
  const a = new Date(from * 1000);
  const b = new Date(to * 1000);
  const span = to - from;
  let shape: keyof typeof RANGES;
  if (span < 2 * DAY) {
    const today = new Date().toDateString();
    const dated = a.toDateString() !== today || b.toDateString() !== today;
    const seconds = span < 10 * MINUTE;
    shape = dated ? (seconds ? "datedClockSeconds" : "datedClock") : seconds ? "clockSeconds" : "clock";
  } else {
    const thisYear = new Date().getFullYear();
    shape = a.getFullYear() === thisYear && b.getFullYear() === thisYear ? "dates" : "datesWithYears";
  }
  return ranges(locale, shape).formatRange(a, b);
}
