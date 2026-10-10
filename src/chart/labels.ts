// Time labels on the chart: the axis, the hovered candle, a measured stretch.

const MINUTE = 60;
const DAY = 86_400;

const pad2 = (n: number) => String(n).padStart(2, "0");
const hm = (d: Date) => `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
const hms = (d: Date) => `${hm(d)}:${pad2(d.getSeconds())}`;

function isToday(d: Date): boolean {
  return d.toDateString() === new Date().toDateString();
}

/** Time axis labels; called for every label on every frame, so no Intl. */
export function axisLabel(t: number, span: number): string {
  const d = new Date(t * 1000);
  // Midnight, and every label of a multi-day view, reads as a date.
  if (span > 2 * DAY || (d.getHours() === 0 && d.getMinutes() === 0 && d.getSeconds() === 0)) {
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }
  // Labels land on whole minutes beyond about five minutes of view.
  return span <= 5 * MINUTE ? hms(d) : hm(d);
}

/** A candle's start. Daily candles carry the date of the day they open, where they open. */
export function candleLabel(t: number, secs: number, dayOffset: number): string {
  if (secs >= DAY) {
    const day = new Date((t + dayOffset) * 1000);
    return `${day.getUTCFullYear()}/${day.getUTCMonth() + 1}/${day.getUTCDate()}`;
  }
  const d = new Date(t * 1000);
  const time = secs < MINUTE ? hms(d) : hm(d);
  return isToday(d) ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`;
}
