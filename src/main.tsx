import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

const root = ReactDOM.createRoot(document.getElementById("root")!);
const render = (content: React.ReactNode) => root.render(<React.StrictMode>{content}</React.StrictMode>);
const designerMode = import.meta.env.DEV && new URLSearchParams(window.location.search).has("designer");

if (designerMode) {
  void import("./components/DesignPlayground").then(({ DesignPlayground }) => render(<DesignPlayground />));
} else {
  render(<App />);
}
