import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Acct, APP_PASSWORD_LENGTH, cleanAppPassword, looksLikeEmail } from "./accounts";

const APP_PASSWORDS_URL = "https://myaccount.google.com/apppasswords";

/** Adds Gmail with a Google app password (IMAP + SMTP). Both servers are checked before anything is saved. */
export default function AppPasswordForm({ onAdded, onCancel }: { onAdded: (a: Acct) => void; onCancel?: () => void }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const code = cleanAppPassword(password);
  const ready = looksLikeEmail(email) && code.length === APP_PASSWORD_LENGTH;

  async function connect(e: React.FormEvent) {
    e.preventDefault();
    if (!ready || busy) return;
    setBusy(true); setError("");
    try {
      const a = await invoke<Acct>("add_imap_account", { email: email.trim(), password: code });
      setPassword("");
      onAdded(a);
    } catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  }

  return (
    <form className="form apppw" onSubmit={connect}>
      <b>Gmail with an app password</b>
      <ol className="muted apppw-help">
        <li>Your Google account needs <b>2-Step Verification</b> switched on.</li>
        <li>Create an app password at{" "}
          <a href={APP_PASSWORDS_URL} onClick={(ev) => { ev.preventDefault(); invoke("open_link", { url: APP_PASSWORDS_URL }).catch(() => {}); }}>myaccount.google.com/apppasswords</a>
          {" "}(any name will do, for example “Graphite Mail”).</li>
        <li>Paste the 16-letter code below. Spaces are removed for you.</li>
      </ol>
      <label>Gmail address
        <input type="email" autoComplete="username" placeholder="name@gmail.com" value={email} disabled={busy} onChange={(e) => setEmail(e.target.value)} />
      </label>
      <label>App password
        <input type="password" autoComplete="off" spellCheck={false} placeholder="abcd efgh ijkl mnop" value={password} disabled={busy}
          onChange={(e) => setPassword(e.target.value)} />
      </label>
      {password && code.length !== APP_PASSWORD_LENGTH && (
        <div className="muted" style={{ fontSize: 12 }}>{code.length} of {APP_PASSWORD_LENGTH} letters</div>
      )}
      <div className="muted" style={{ fontSize: 12 }}>The app password is kept in your system keyring, not in Graphite Mail's files. You can revoke it at any time on the same Google page.</div>
      <div className="rowbtns">
        <button className="primary" type="submit" disabled={!ready || busy}>{busy ? "Checking with Gmail…" : "Connect"}</button>
        {onCancel && <button type="button" disabled={busy} onClick={onCancel}>Cancel</button>}
      </div>
      {error && <div className="error" role="alert">{error}</div>}
    </form>
  );
}
