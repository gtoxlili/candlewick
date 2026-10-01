import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import HoldingsApp from "./HoldingsApp";
import { setupPlatform } from "@/lib/platform";
import "../index.css";

setupPlatform();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <HoldingsApp />
  </StrictMode>,
);
