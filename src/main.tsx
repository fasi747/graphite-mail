import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { invoke } from "@tauri-apps/api/core";

const report = (m: string) => { try { invoke("log_js", { message: m }); } catch { /* ignore */ } };
window.addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled: ${e.reason}`));

class Boundary extends React.Component<{ children: React.ReactNode }, { err: string | null }> {
  state = { err: null as string | null };
  static getDerivedStateFromError(e: Error) { return { err: `${e.message}\n${e.stack ?? ""}` }; }
  componentDidCatch(e: Error) { report(`render error: ${e.message}\n${e.stack}`); }
  render() {
    return this.state.err
      ? <div style={{ padding: 30, fontFamily: "monospace", whiteSpace: "pre-wrap", color: "#e5484d" }}>
          <b>Something went wrong — please send this to the developer:</b>{"\n\n"}{this.state.err}
          {"\n\n"}<button onClick={() => location.reload()}>Reload</button>
        </div>
      : this.props.children;
  }
}

import "@fontsource-variable/inter";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Boundary><App /></Boundary>
  </React.StrictMode>,
);
