import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { TrayPopup } from "./components/tray/TrayPopup";
import "./styles/tokens.css";
import "./styles/base.css";

// The tray popup loads this same bundle with ?window=tray and mounts its own small tree.
const target = new URLSearchParams(location.search).get("window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{target === "tray" ? <TrayPopup /> : <App />}</React.StrictMode>,
);
