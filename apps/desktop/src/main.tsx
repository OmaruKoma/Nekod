import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import HudOverlay from "./HudOverlay";
import { I18nProvider } from "./i18n";
import "./styles.css";

const isHudOverlay =
  new URLSearchParams(window.location.search).get("overlay") === "hud";

if (isHudOverlay) {
  document.documentElement.classList.add("hud-overlay-mode");
  document.body.classList.add("hud-overlay-mode");
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isHudOverlay ? (
      <HudOverlay />
    ) : (
      <I18nProvider>
        <App />
      </I18nProvider>
    )}
  </React.StrictMode>,
);
