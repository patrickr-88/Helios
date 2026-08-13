import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import "./styles/app.css";

// StrictMode double-invokes effects in development, which is exactly the
// pressure the scan-event subscriptions should survive.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
