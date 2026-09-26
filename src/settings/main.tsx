import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { setupPlatform } from "@/lib/platform";
import "../index.css";

setupPlatform();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
