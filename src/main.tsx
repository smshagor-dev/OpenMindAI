import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { AppUpdateMonitor } from "./components/AppUpdateMonitor";
import { StartupOverlay } from "./components/StartupOverlay";
import "./styles.css";
import "./startup.css";
import "./multimodal.css";
import "./projects.css";
import "./work.css";
import "./chat-layout-fix.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppUpdateMonitor />
    <App />
    <StartupOverlay />
  </React.StrictMode>,
);
