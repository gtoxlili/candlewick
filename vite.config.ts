import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

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
  build: {
    // Only ever runs in this Mac's WKWebView (macOS 27).
    target: "safari26",
    minify: "oxc",
    cssMinify: "lightningcss",
    sourcemap: false,
  },
});
