import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

// Each platform builds its own pages: the Tauri CLI names the target for its
// before-build and before-dev commands; a plain `vite` builds for this machine.
const target = process.env.TAURI_ENV_PLATFORM ?? process.platform;
const platform = target === "windows" || target === "win32" ? "windows" : "macos";

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [
    // React Compiler through the oxc native transform: automatic memoization,
    // no hand-written useMemo/useCallback.
    react({ compiler: true }),
    tailwindcss(),
  ],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  // Tauri's dev server expectations: fixed port, no auto-switch, and keep
  // Rust errors visible in the terminal.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  define: {
    // A literal in every module, so the other platform's code, and modules
    // only it imports, are gone before bundling. An exported constant would
    // be folded too late to keep those imports out.
    __WINDOWS__: JSON.stringify(platform === "windows"),
  },
  build: {
    // WKWebView on macOS 27; WebView2 on Windows, whose installer requires
    // the Chromium 125 runtime or later.
    target: platform === "windows" ? "chrome125" : "safari26",
    minify: "oxc",
    cssMinify: "lightningcss",
    sourcemap: false,
    // One page per window; each loads only its own code (the chart library
    // stays out of the settings window).
    rolldownOptions: {
      input: {
        settings: path.resolve(import.meta.dirname, "index.html"),
        chart: path.resolve(import.meta.dirname, "chart.html"),
        holdings: path.resolve(import.meta.dirname, "holdings.html"),
      },
    },
  },
});
