import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import HoldingsApp from "./HoldingsApp";
import { setupPlatform } from "@/lib/platform";
import "../index.css";

// In a plain browser (no app behind the page), answer the IPC from fixtures.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installMocks } = await import("@/dev/mock");
  installMocks("holdings");
}

setupPlatform();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <HoldingsApp />
  </StrictMode>,
);
