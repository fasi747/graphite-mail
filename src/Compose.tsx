import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";

export type ComposeInit = {
  mode: "new" | "reply" | "replyAll" | "forward";
  replyTo?: string;
  to?: string[];
  cc?: string[];
  subject?: string;
  quoteHtml?: string;
  quoteHeader?: string;
  /** Account (email) to send from. */
  from?: string;
};
type Acct = { email: string; display_name: string };

const split = (s: string) => s.split(/[,;\s]+/).map((x) => x.trim()).filter(Boolean);
const valid = (e: string) => /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(e);

export default function Compose({ init, accounts, onClose, onSent }: { init: ComposeInit; accounts: Acct[]; onClose: () => void; onSent: (queued: boolean) => void }) {
  const [from, setFrom] = useState(init.from ?? accounts[0]?.email ?? "");
  const [to, setTo] = useState((init.to ?? []).join(", "));
  const [cc, setCc] = useState((init.cc ?? []).join(", "));
  const [subject, setSubject] = useState(init.subject ?? "");
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);
  const body = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (body.current && init.quoteHtml) {
      body.current.innerHTML =
        `<p><br></p><p><br></p><hr><p style="color:#888">${init.quoteHeader ?? ""}</p>` +
        `<blockquote style="margin:0 0 0 8px;padding-left:10px;border-left:3px solid #ccc">${DOMPurify.sanitize(init.quoteHtml, { FORBID_TAGS: ["style", "script"] })}</blockquote>`;
    }
    (init.mode === "forward" || !init.to?.length ? document.getElementById("cmp-to") : body.current)?.focus();
  }, []);

  async function send() {
    const toList = split(to), ccList = split(cc);
    const bad = [...toList, ...ccList].filter((e) => !valid(e));
    if (!toList.length) return setError("Add at least one recipient.");
    if (bad.length) return setError(`Check these addresses: ${bad.join(", ")}`);
    setSending(true);
    try {
      const html = `<div style="font-family:Calibri,Arial,sans-serif;font-size:11pt">${body.current?.innerHTML ?? ""}</div>`;
      const sent = await invoke<boolean>("send_mail", {
        draft: { mode: init.mode, reply_to: init.replyTo ?? null, from: from || null, to: toList, cc: ccList, subject, html },
      });
      onSent(!sent);
    } catch (e) {
      setError(String(e));
      setSending(false);
    }
  }

  const cmd = (c: string) => document.execCommand(c);

  return (
    <div className="sheet-backdrop" onKeyDown={(e) => { if (e.key === "Escape") onClose(); if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) send(); }}>
      <div className="sheet">
        <div className="sheet-bar">
          <button onClick={onClose}>Cancel</button>
          <b>{subject || "New Message"}</b>
          <button className="primary" onClick={send} disabled={sending}>{sending ? "Sending…" : "Send"}</button>
        </div>
        {accounts.length > 1 && (
          <label className="field"><span>From:</span>
            <select value={from} onChange={(e) => setFrom(e.target.value)} disabled={!!init.replyTo} title={init.replyTo ? "Replies and forwards are sent from the account that received the message" : undefined}>
              {accounts.map((a) => <option key={a.email} value={a.email}>{a.display_name} &lt;{a.email}&gt;</option>)}
            </select>
          </label>
        )}
        <label className="field"><span>To:</span><input id="cmp-to" value={to} onChange={(e) => setTo(e.target.value)} /></label>
        <label className="field"><span>Cc:</span><input value={cc} onChange={(e) => setCc(e.target.value)} /></label>
        <label className="field"><span>Subject:</span><input value={subject} onChange={(e) => setSubject(e.target.value)} /></label>
        <div className="fmt">
          <button className="icon" onClick={() => cmd("bold")}><b>B</b></button>
          <button className="icon" onClick={() => cmd("italic")}><i>I</i></button>
          <button className="icon" onClick={() => cmd("underline")}><u>U</u></button>
          <button className="icon" onClick={() => cmd("insertUnorderedList")}>•</button>
        </div>
        <div className="editor" contentEditable ref={body} suppressContentEditableWarning />
        {error && <div className="error" style={{ padding: "0 16px 12px" }}>{error}</div>}
      </div>
    </div>
  );
}
