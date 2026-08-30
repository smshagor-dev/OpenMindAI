import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { AppUpdateMonitor } from "./components/AppUpdateMonitor";
import { bootstrapPlatformDataset } from "./lib/platform";
import "./styles.css";
import "./multimodal.css";
import "./projects.css";
import "./work.css";
import "./mobile.css";
import "./mobile-platform.css";

bootstrapPlatformDataset();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppUpdateMonitor />
    <App />
  </React.StrictMode>,
);
