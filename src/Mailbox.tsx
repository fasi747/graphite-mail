import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import DOMPurify from "dompurify";

type Account = { email: string; display_name: string };
type Folder = { id: string; name: string; well_known: string | null; unread: number; total: number; parent_remote_id: string | null };
type Msg = {
  id: string; subject: string | null; sender_name: string | null; sender_address: string | null;
  snippet: string | null; received_at: number; is_read: boolean; is_flagged: boolean; has_attachments: boolean;
};
type Detail = Msg & { to_json: string | null; cc_json: string | null; body_html: string | null };

const PAGE = 200;

function fmtDate(ts: number) {
  const d = new Date(ts * 1000);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString([], { day: "numeric", month: "short", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" });
}

function people(json: string | null) {
  try {
    return (JSON.parse(json || "[]") as { name?: string; address?: string }[]).map((p) => p.name || p.address).join(", ");
  } catch {
    return "";
  }
}

/** Sanitize, block remote content unless allowed, and optionally darken the email canvas. */
function buildDoc(html: string, allowRemote: boolean, dark: boolean) {
  let blocked = 0;
  DOMPurify.removeAllHooks();
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A") {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
    if (!allowRemote) {
      for (const attr of ["src", "background", "srcset"]) {
        const v = node.getAttribute(attr);
        if (v && /^(https?:)?\/\//i.test(v)) {
          node.removeAttribute(attr);
          blocked++;
        }
      }
      const style = node.getAttribute("style");
      if (style && /url\(/i.test(style)) {
        node.setAttribute("style", style.replace(/url\([^)]*\)/gi, "none"));
        blocked++;
      }
    }
  });
  const clean = DOMPurify.sanitize(html, { WHOLE_DOCUMENT: true, FORBID_TAGS: ["script", "form", "iframe", "object", "embed"] });
  const darkCss = dark ? "html{filter:invert(.9) hue-rotate(180deg);background:#fff} img,video,picture{filter:invert(1) hue-rotate(180deg)}" : "";
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: cid: ${allowRemote ? "https: http:" : ""}; style-src 'unsafe-inline';"><style>body{font-family:system-ui,sans-serif;font-size:14px;margin:16px 20px;word-wrap:break-word} ${darkCss}</style></head><body>${clean}</body></html>`;
  return { doc, blocked };
}

export default function Mailbox({ account, onSignOut }: { account: Account; onSignOut: () => void }) {
  const [folders, setFolders] = useState<Folder[]>([]);
  const [folderId, setFolderId] = useState<string | null>(null);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [selected, setSelected] = useState<Detail | null>(null);
  const [query, setQuery] = useState("");
  const [syncing, setSyncing] = useState(false);
  const [status, setStatus] = useState("");
  const [online, setOnline] = useState(navigator.onLine);
  const [allowRemote, setAllowRemote] = useState(false);
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    (localStorage.getItem("theme") as "light" | "dark") || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"),
  );
  const [darkMail, setDarkMail] = useState(true);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem("theme", theme); } catch { /* ignore */ }
  }, [theme]);

  const loadFolders = async () => {
    const f = await invoke<Folder[]>("list_folders");
    setFolders(f);
    setFolderId((cur) => cur ?? f.find((x) => x.well_known === "inbox")?.id ?? f[0]?.id ?? null);
  };

  const loadMessages = async () => {
    if (query.trim()) setMsgs(await invoke<Msg[]>("search_messages", { query }));
    else if (folderId) setMsgs(await invoke<Msg[]>("list_messages", { folderId, offset: 0, limit: PAGE }));
  };

  const syncNow = async () => {
    setSyncing(true);
    setStatus("Syncing…");
    try {
      const r = await invoke<{ folders: number; messages_changed: number; bodies_downloaded: number }>("sync_now");
      setStatus(`Synced · ${r.messages_changed} changes`);
      await loadFolders();
      await loadMessages();
    } catch (e) {
      setStatus(navigator.onLine ? `Sync failed: ${e}` : "Offline – showing saved mail");
    } finally {
      setSyncing(false);
    }
  };

  useEffect(() => {
    loadFolders().then(syncNow);
    const un = listen("sync-finished", () => { loadFolders(); loadMessages(); });
    const on = () => setOnline(true), off = () => setOnline(false);
    addEventListener("online", on);
    addEventListener("offline", off);
    return () => { un.then((f) => f()); removeEventListener("online", on); removeEventListener("offline", off); };
  }, []);

  useEffect(() => {
    const t = setTimeout(loadMessages, query ? 150 : 0);
    return () => clearTimeout(t);
  }, [folderId, query]);

  const loadMore = async () => {
    if (query.trim() || !folderId || msgs.length % PAGE !== 0 || msgs.length === 0) return;
    const more = await invoke<Msg[]>("list_messages", { folderId, offset: msgs.length, limit: PAGE });
    if (more.length) setMsgs((m) => [...m, ...more]);
  };

  const virt = useVirtualizer({ count: msgs.length, getScrollElement: () => scroller.current, estimateSize: () => 76, overscan: 10 });
  const items = virt.getVirtualItems();
  useEffect(() => {
    const last = items[items.length - 1];
    if (last && last.index >= msgs.length - 20) loadMore();
  }, [items]);

  const open = async (m: Msg) => {
    setAllowRemote(false);
    setSelected(await invoke<Detail>("get_message", { id: m.id }));
  };

  const rendered = useMemo(
    () => (selected?.body_html ? buildDoc(selected.body_html, allowRemote, theme === "dark" && darkMail) : null),
    [selected, allowRemote, theme, darkMail],
  );

  return (
    <div className="shell">
      <nav className="rail">
        <div className="brand">Graphite Mail</div>
        <div className="account" title={account.email}>{account.email}</div>
        <div className="folders">
          {folders.map((f) => (
            <div key={f.id} className={`folder ${f.id === folderId && !query ? "active" : ""}`} onClick={() => { setQuery(""); setFolderId(f.id); setSelected(null); }}>
              <span>{f.name}</span>
              {f.unread > 0 && <span className="count">{f.unread}</span>}
            </div>
          ))}
          {!folders.length && <div className="empty">No folders yet</div>}
        </div>
        <div className="status">
          <span className={`dot ${online ? "" : "off"}`} />
          <span style={{ flex: 1 }}>{online ? status || "Online" : "Offline – saved mail"}</span>
          <button onClick={syncNow} disabled={syncing || !online} title="Sync now">⟳</button>
          <button onClick={() => setTheme(theme === "dark" ? "light" : "dark")} title="Toggle theme">{theme === "dark" ? "☀" : "☾"}</button>
          <button onClick={onSignOut} title="Sign out">⎋</button>
        </div>
      </nav>

      <section className="list">
        <div className="search">
          <input placeholder="Search all saved mail…" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <div className="scroller" ref={scroller}>
          {msgs.length === 0 ? (
            <div className="empty">{query ? "No matches" : syncing ? "Downloading mail…" : "No messages"}</div>
          ) : (
            <div style={{ height: virt.getTotalSize(), position: "relative" }}>
              {items.map((v) => {
                const m = msgs[v.index];
                return (
                  <div key={m.id} className={`msg ${m.is_read ? "" : "unread"} ${selected?.id === m.id ? "active" : ""}`}
                    style={{ top: v.start }} onClick={() => open(m)}>
                    <div className="top">
                      <span className="from">{m.sender_name || m.sender_address || "(no sender)"}</span>
                      <span className="date">{m.has_attachments ? "📎 " : ""}{m.is_flagged ? "⚑ " : ""}{fmtDate(m.received_at)}</span>
                    </div>
                    <div className="subj">{m.subject || "(no subject)"}</div>
                    <div className="snip">{m.snippet}</div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </section>

      <section className="reader">
        {selected ? (
          <>
            <header>
              <h2>{selected.subject || "(no subject)"}</h2>
              <div className="meta">
                <div><b>{selected.sender_name}</b> &lt;{selected.sender_address}&gt;</div>
                {people(selected.to_json) && <div>To: {people(selected.to_json)}</div>}
                {people(selected.cc_json) && <div>Cc: {people(selected.cc_json)}</div>}
                <div>{new Date(selected.received_at * 1000).toLocaleString()}</div>
              </div>
              {theme === "dark" && (
                <div className="tools">
                  <button onClick={() => setDarkMail(!darkMail)}>{darkMail ? "☀ Light email" : "☾ Dark email"}</button>
                </div>
              )}
            </header>
            {rendered && rendered.blocked > 0 && !allowRemote && (
              <div className="notice">
                <span>Remote images blocked to protect your privacy.</span>
                <button onClick={() => setAllowRemote(true)}>Show images</button>
              </div>
            )}
            {rendered ? (
              <iframe sandbox="allow-popups allow-popups-to-escape-sandbox" srcDoc={rendered.doc} title="message" />
            ) : (
              <div className="empty">This message hasn't been downloaded for offline reading yet. It will appear after the next sync.</div>
            )}
          </>
        ) : (
          <div className="empty">Select a message to read</div>
        )}
      </section>
    </div>
  );
}
