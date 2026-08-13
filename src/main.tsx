import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
// Used by the Paperlike theme. Latin subsets only: the full package ships a
// @font-face per script (16 per weight), which dominated the CSS bundle.
import "@fontsource/playpen-sans/latin-400.css";
import "@fontsource/playpen-sans/latin-500.css";
import "@fontsource/playpen-sans/latin-600.css";
import "@fontsource/playpen-sans/latin-700.css";
import App from "./App";

document.documentElement.dataset.os = navigator.userAgent.includes("Windows")
  ? "windows"
  : navigator.userAgent.includes("Mac")
    ? "macos"
    : "linux";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
