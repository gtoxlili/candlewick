// Chart colors read from the page's CSS (index.css), so the canvas follows
// the palette, the light/dark appearance and the red/green convention.

export type Rgb = [number, number, number];

export interface ChartColors {
  /** The accent (`--glow`), as "rgb(r, g, b)" (Liveline derives its palette from it). */
  accent: string;
  glow: Rgb;
  up: Rgb;
  down: Rgb;
  /** The palette's other hues, for overlays: drawings and indicators. */
  violet: Rgb;
  mint: Rgb;
  amber: Rgb;
  /** Text and lines (`--ink`), opaque; the overlays apply their own alpha. */
  ink: Rgb;
  dark: boolean;
}

let canvas: CanvasRenderingContext2D | null = null;

/** Any CSS color (system colors and var() included) → sRGB channels. */
function channels(css: string, probe: HTMLElement): Rgb {
  probe.style.color = "";
  probe.style.color = css;
  const computed = getComputedStyle(probe).color;
  // Normalize whatever syntax the engine serializes (rgb, oklch, color()…)
  // by painting one pixel and reading it back.
  canvas ??= (() => {
    const el = document.createElement("canvas");
    el.width = el.height = 1;
    return el.getContext("2d", { willReadFrequently: true })!;
  })();
  canvas.clearRect(0, 0, 1, 1);
  canvas.fillStyle = "#000";
  canvas.fillStyle = computed;
  canvas.fillRect(0, 0, 1, 1);
  const [r, g, b] = canvas.getImageData(0, 0, 1, 1).data;
  return [r, g, b];
}

export function readChartColors(): ChartColors {
  const probe = document.createElement("span");
  probe.style.display = "none";
  document.body.append(probe);
  try {
    const glow = channels("var(--primary)", probe);
    return {
      accent: `rgb(${glow.join(", ")})`,
      glow,
      up: channels("var(--up)", probe),
      down: channels("var(--down)", probe),
      violet: channels("var(--violet)", probe),
      mint: channels("var(--mint)", probe),
      amber: channels("var(--amber)", probe),
      ink: channels("var(--ink)", probe),
      dark: matchMedia("(prefers-color-scheme: dark)").matches,
    };
  } finally {
    probe.remove();
  }
}

export const sameColors = (a: ChartColors, b: ChartColors) =>
  a.accent === b.accent &&
  a.dark === b.dark &&
  a.up.join() === b.up.join() &&
  a.down.join() === b.down.join() &&
  a.violet.join() === b.violet.join() &&
  a.ink.join() === b.ink.join();

/** `rgba(r, g, b, a)` */
export const rgba = ([r, g, b]: Rgb, a: number): string => `rgba(${r}, ${g}, ${b}, ${a})`;
