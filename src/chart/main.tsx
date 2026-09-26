import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import ChartApp from "./ChartApp";
import { setupPlatform } from "@/lib/platform";
import "../index.css";

setupPlatform();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ChartApp />
  </StrictMode>,
);
