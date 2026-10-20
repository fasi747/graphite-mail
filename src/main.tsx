import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { invoke } from "@tauri-apps/api/core";

const report = (m: string) => { try { invoke("log_js", { message: m }); } catch { /* ignore */ } };
window.addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
// A blocked resource (content-security-policy) is reported to the terminal, so a break is noticed at once.
document.addEventListener("securitypolicyviolation", (e) => report(`CSP blocked ${e.violatedDirective}: ${e.blockedURI} (${e.sourceFile ?? "page"}:${e.lineNumber})`));
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
import "@fontsource-variable/lora";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Boundary><App /></Boundary>
  </React.StrictMode>,
);
