import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import Mailbox from "./Mailbox";
import "./App.css";

type Account = { email: string; display_name: string };
type SetupState = { ready: boolean; default_dir: string };

try {
  const t = localStorage.getItem("theme");
  document.documentElement.dataset.theme = t || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
} catch { /* ignore */ }

function SetupWizard({ defaultDir, onDone }: { defaultDir: string; onDone: () => void }) {
  const [dir, setDir] = useState(defaultDir);
  const [error, setError] = useState("");

  async function browse() {
    const picked = await open({ directory: true, defaultPath: dir, title: "Choose where Graphite Mail stores your mail" });
    if (typeof picked === "string") setDir(picked.endsWith("graphite-mail") ? picked : `${picked}/graphite-mail`);
  }

  async function confirm() {
    setError("");
    try {
      await invoke("initialize_storage", { path: dir });
      onDone();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <main className="centered">
      <h1>Welcome to Graphite Mail</h1>
      <p className="muted">Choose where your offline mail and calendar will be stored. You can move it later.</p>
      <div className="row">
        <input value={dir} onChange={(e) => setDir(e.target.value)} />
        <button onClick={browse}>Browse…</button>
      </div>
      <button className="primary" onClick={confirm}>Continue</button>
      {error && <p className="error">{error}</p>}
    </main>
  );
}

export default function App() {
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [account, setAccount] = useState<Account | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const loadSetup = () => invoke<SetupState>("setup_state").then(setSetup).catch((e) => setError(String(e)));

  useEffect(() => {
    loadSetup();
    invoke<Account | null>("current_account").then(setAccount);
  }, []);

  async function signIn() {
    setBusy(true);
    setError("");
    try {
      setAccount(await invoke<Account>("sign_in"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function signOut() {
    await invoke("sign_out");
    setAccount(null);
  }

  if (!setup) return <main className="centered">{error || "Loading…"}</main>;
  if (!setup.ready) return <SetupWizard defaultDir={setup.default_dir} onDone={loadSetup} />;
  if (account) return <Mailbox account={account} onSignOut={signOut} />;

  return (
    <main className="centered">
      <h1>Graphite Mail</h1>
      <p className="muted">Offline-first mail &amp; calendar for Microsoft 365.</p>
      <button className="primary" onClick={signIn} disabled={busy}>
        {busy ? "Waiting for browser sign-in…" : "Sign in with Microsoft"}
      </button>
      {busy && <p className="muted">Finish signing in in your web browser, then come back here.</p>}
      {error && <p className="error">{error}</p>}
    </main>
  );
}
