import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { SettingsWindow } from "./components/SettingsWindow";
import "./App.css";

const rootElement = document.getElementById("root");

if (rootElement === null) {
  throw new Error("TULE could not find its application root element.");
}

const windowLabel = getCurrentWindow().label;
const surface = windowLabel === "settings" ? <SettingsWindow /> : <App />;

ReactDOM.createRoot(rootElement).render(<React.StrictMode>{surface}</React.StrictMode>);
