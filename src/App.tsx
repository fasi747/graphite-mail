import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Account = { email: string; display_name: string };

export default function App() {
  const [account, setAccount] = useState<Account | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
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

  return (
    <main className="container">
      <h1>Graphite Mail</h1>
      {account ? (
        <>
          <p>
            Signed in as <b>{account.display_name}</b> ({account.email})
          </p>
          <button onClick={signOut}>Sign out</button>
        </>
      ) : (
        <>
          <p>Offline-first mail &amp; calendar for Microsoft 365.</p>
          <button onClick={signIn} disabled={busy}>
            {busy ? "Waiting for browser sign-in…" : "Sign in with Microsoft"}
          </button>
        </>
      )}
      {error && <p style={{ color: "#c33" }}>{error}</p>}
    </main>
  );
}
