import React, { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import DOMPurify from "dompurify";
import { DragBar, TrafficLights } from "./WindowControls";
import Icon from "./Icon";
import Compose, { ComposeInit } from "./Compose";
import CalendarView, { Cal } from "./CalendarView";
import { ask } from "@tauri-apps/plugin-dialog";

type Account = { email: string; display_name: string };
type Folder = { id: string; name: string; well_known: string | null; unread: number; total: number; parent_remote_id: string | null };
type Msg = {
  id: string; subject: string | null; sender_name: string | null; sender_address: string | null;
  snippet: string | null; received_at: number; is_read: boolean; is_flagged: boolean; has_attachments: boolean;
};
type QueueItem = { seq: number; what: string; failed: boolean; tries: number; error: string | null };
type Detail = Msg & { to_json: string | null; cc_json: string | null; body_html: string | null };

type SyncReport = { messages_changed: number; replay_error: string | null; calendar_error: string | null };
const PAGE = 200;

/** One plain-language line describing how the last sync went. */
function syncStatus(r: SyncReport) {
  if (r.replay_error) return `Couldn't send your changes: ${r.replay_error}`;
  if (r.calendar_error) return `Mail is up to date, but the calendar failed: ${r.calendar_error}`;
  return r.messages_changed ? `Synced · ${r.messages_changed} new` : "Up to date";
}

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
      // A click navigates the email frame; the app (Rust side) intercepts that and opens the system browser.
      node.setAttribute("target", "_self");
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

/** Keeps a crash in one pane from taking the whole app down, and says what happened in plain words. */
class PaneBoundary extends React.Component<{ children: React.ReactNode }, { err: string | null }> {
  state = { err: null as string | null };
  static getDerivedStateFromError(e: Error) { return { err: e.message }; }
  componentDidCatch(e: Error) { invoke("log_js", { message: `calendar crashed: ${e.message}\n${e.stack}` }); }
  render() {
    return this.state.err
      ? <section className="calendar"><div className="empty">The calendar couldn't be shown ({this.state.err}).<br /><button onClick={() => this.setState({ err: null })}>Try again</button></div></section>
      : this.props.children;
  }
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
  const [compose, setCompose] = useState<ComposeInit | null>(null);
  const [toast, setToast] = useState("");
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [queueOpen, setQueueOpen] = useState(false);
  const pending = queue.filter((q) => !q.failed).length;
  const rejected = queue.filter((q) => q.failed).length;
  const refreshQueue = () => invoke<QueueItem[]>("pending_details").then(setQueue).catch(() => {});
  const [moveOpen, setMoveOpen] = useState(false);
  const [view, setView] = useState<"mail" | "calendar">("mail");
  const [cals, setCals] = useState<Cal[]>([]);
  const [hiddenCals, setHiddenCals] = useState<Set<string>>(() => { try { return new Set(JSON.parse(localStorage.getItem("hiddenCals") || "[]")); } catch { return new Set(); } });
  useEffect(() => { try { localStorage.setItem("hiddenCals", JSON.stringify([...hiddenCals])); } catch { /* ignore */ } }, [hiddenCals]);
  const loadCals = () => invoke<Cal[]>("list_calendars").then(setCals).catch(() => {});
  const viewRef = useRef({ folderId: null as string | null, query: "" });
  viewRef.current = { folderId, query };

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem("theme", theme); } catch { /* ignore */ }
  }, [theme]);

  const loadFolders = async () => {
    loadCals();
    const f = await invoke<Folder[]>("list_folders");
    setFolders(f);
    setFolderId((cur) => cur ?? f.find((x) => x.well_known === "inbox")?.id ?? f[0]?.id ?? null);
  };

  const loadMessages = async () => {
    const { folderId, query } = viewRef.current;
    if (query.trim()) setMsgs(await invoke<Msg[]>("search_messages", { query }));
    else if (folderId) setMsgs(await invoke<Msg[]>("list_messages", { folderId, offset: 0, limit: PAGE }));
    refreshQueue();
  };

  const flash = (t: string) => { setToast(t); setTimeout(() => setToast(""), 2600); };
  const patch = (id: string, p: Partial<Msg>) => {
    setMsgs((ms) => ms.map((m) => (m.id === id ? { ...m, ...p } : m)));
    setSelected((s) => (s && s.id === id ? { ...s, ...p } : s));
  };
  const removeFromList = (id: string) => {
    const idx = msgs.findIndex((m) => m.id === id);
    const next = msgs[idx + 1] ?? msgs[idx - 1];
    setMsgs((ms) => ms.filter((m) => m.id !== id));
    if (next) open(next); else setSelected(null);
  };
  const act = async (fn: () => Promise<unknown>, after?: () => void) => {
    try { await fn(); after?.(); loadFolders(); refreshQueue(); }
    catch (e) { flash(String(e)); }
  };
  const toggleRead = (m: Msg) => act(() => invoke("set_read", { id: m.id, read: !m.is_read }), () => patch(m.id, { is_read: !m.is_read }));
  const toggleFlag = (m: Msg) => act(() => invoke("set_flag", { id: m.id, flagged: !m.is_flagged }), () => patch(m.id, { is_flagged: !m.is_flagged }));
  const del = async (m: Msg) => {
    const inTrash = folders.find((f) => f.id === folderId)?.well_known === "deleteditems";
    if (inTrash && !(await ask("Permanently delete this message? This can't be undone.", { title: "Delete message", kind: "warning" }))) return;
    act(() => invoke("delete_message", { id: m.id }), () => removeFromList(m.id));
  };
  const moveTo = (m: Msg, f: Folder) => { setMoveOpen(false); act(() => invoke("move_message", { id: m.id, folderId: f.id }), () => { removeFromList(m.id); flash(`Moved to ${f.name}`); }); };

  const addrs = (json: string | null) => { try { return (JSON.parse(json || "[]") as { address?: string }[]).map((p) => p.address!).filter(Boolean); } catch { return []; } };
  const startCompose = (mode: ComposeInit["mode"]) => {
    if (mode === "new" || !selected) return setCompose({ mode: "new" });
    const s = selected;
    const me = account.email.toLowerCase();
    const header = `On ${new Date(s.received_at * 1000).toLocaleString()}, ${s.sender_name ?? ""} &lt;${s.sender_address ?? ""}&gt; wrote:`;
    const subj = s.subject ?? "";
    const base = { replyTo: s.id, quoteHtml: s.body_html ?? "", quoteHeader: header };
    if (mode === "forward") return setCompose({ ...base, mode, subject: /^fw(d)?:/i.test(subj) ? subj : `Fw: ${subj}` });
    const re = /^re:/i.test(subj) ? subj : `Re: ${subj}`;
    const to = [s.sender_address ?? ""].filter(Boolean);
    if (mode === "reply") return setCompose({ ...base, mode, to, subject: re });
    const others = [...addrs(s.to_json), ...addrs(s.cc_json)].filter((a) => a.toLowerCase() !== me && !to.includes(a));
    setCompose({ ...base, mode, to, cc: others, subject: re });
  };

  // Keyboard shortcuts (ignored while typing)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (compose || t.tagName === "INPUT" || t.isContentEditable) return;
      const idx = selected ? msgs.findIndex((m) => m.id === selected.id) : -1;
      if (e.key === "ArrowDown" && msgs[idx + 1]) { e.preventDefault(); open(msgs[idx + 1]); }
      else if (e.key === "ArrowUp" && idx > 0) { e.preventDefault(); open(msgs[idx - 1]); }
      else if (!selected) { if ((e.ctrlKey || e.metaKey) && e.key === "n") setCompose({ mode: "new" }); }
      else if (e.key === "Delete") del(selected);
      else if ((e.ctrlKey || e.metaKey) && e.key === "n") setCompose({ mode: "new" });
      else if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "r") startCompose("replyAll");
      else if ((e.ctrlKey || e.metaKey) && e.key === "r") { e.preventDefault(); startCompose("reply"); }
      else if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "f") startCompose("forward");
      else if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "u") toggleRead(selected);
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  });

  const syncNow = async () => {
    setSyncing(true);
    setStatus("Syncing…");
    try {
      const r = await invoke<SyncReport>("sync_now");
      setStatus(syncStatus(r));
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
    const un = listen<{ Ok?: SyncReport; Err?: string }>("sync-finished", (e) => {
      if (e.payload.Ok) setStatus(syncStatus(e.payload.Ok));
      else if (e.payload.Err) setStatus(`Sync failed: ${e.payload.Err}`);
      loadFolders(); loadMessages();
    });
    // Links clicked inside an email are handled by the app shell (Rust) and reported here.
    const un2 = listen<string>("mailto", (e) => {
      const to = decodeURIComponent(e.payload.slice(7).split("?")[0]);
      setCompose({ mode: "new", to: to ? [to] : [] });
    });
    const un3 = listen<string>("link-failed", () => flash("Couldn't open the link in your web browser."));
    const on = () => setOnline(true), off = () => setOnline(false);
    addEventListener("online", on);
    addEventListener("offline", off);
    return () => { un.then((f) => f()); un2.then((f) => f()); un3.then((f) => f()); removeEventListener("online", on); removeEventListener("offline", off); };
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
    setMoveOpen(false);
    const d = await invoke<Detail>("get_message", { id: m.id });
    setSelected(d);
    if (!m.is_read) {
      invoke("set_read", { id: m.id, read: true }).then(() => { patch(m.id, { is_read: true }); loadFolders(); }).catch(() => {});
    }
  };

  const rendered = useMemo(
    () => (selected?.body_html ? buildDoc(selected.body_html, allowRemote, theme === "dark" && darkMail) : null),
    [selected, allowRemote, theme, darkMail],
  );

  const current = folders.find((f) => f.id === folderId);
  const initials = (n?: string | null) => (n || "?").split(/\s+/).map((w) => w[0]).slice(0, 2).join("").toUpperCase();

  return (
    <div className="shell">
      <nav className="sidebar">
        <DragBar><TrafficLights /></DragBar>
        <div className={`folder navitem ${view === "calendar" ? "active" : ""}`} onClick={() => setView("calendar")}>
          <span className="ico"><Icon name="calendar" /></span><span className="name">Calendar</span>
        </div>
        <div className="section">Mailboxes</div>
        <div className="folders">
          {folders.map((f) => (
            <div key={f.id} className={`folder ${f.id === folderId && !query && view === "mail" ? "active" : ""}`} onClick={() => { setView("mail"); setQuery(""); setFolderId(f.id); setSelected(null); }}>
              <span className="ico"><Icon name={f.well_known ?? "folder"} /></span>
              <span className="name">{f.name}</span>
              {f.unread > 0 && <span className="count">{f.unread}</span>}
            </div>
          ))}
          {!folders.length && <div className="empty">No folders yet</div>}
          {cals.length > 0 && <div className="section" style={{ padding: "14px 10px 4px" }}>Calendars</div>}
          {cals.map((c) => (
            <div key={c.id} className="folder calrow" onClick={() => setView("calendar")} title="Open calendar">
              <input type="checkbox" checked={!hiddenCals.has(c.id)} style={{ accentColor: c.color }} title="Show or hide this calendar"
                onClick={(e) => e.stopPropagation()}
                onChange={() => setHiddenCals((h) => { const n = new Set(h); n.has(c.id) ? n.delete(c.id) : n.add(c.id); return n; })} />
              <span className="name">{c.name}</span>
            </div>
          ))}
        </div>
        <div className="account">
          <div className="avatar">{initials(account.display_name)}</div>
          <div className="who">
            <div>{account.display_name}</div>
            <div className="muted" title={status}><span className={`dot ${online ? "" : "off"}`} />{online ? status || "Online" : "Offline – saved mail"}</div>
            {queue.length > 0 && (
              <div className="muted queue-link" onClick={() => { refreshQueue(); setQueueOpen(true); }} title="See what is waiting">
                {pending > 0 ? `${pending} waiting to sync` : ""}{pending > 0 && rejected > 0 ? " · " : ""}{rejected > 0 ? `${rejected} not accepted` : ""}
              </div>
            )}
          </div>
          <button className="icon" onClick={onSignOut} title="Sign out"><Icon name="signout" /></button>
        </div>
      </nav>

      {view === "calendar" && <PaneBoundary><CalendarView calendars={cals} hidden={hiddenCals} onToast={flash} /></PaneBoundary>}
      {view === "mail" && <>
      <section className="list">
        <div className="toolbar" data-tauri-drag-region>
          <div className="title" data-tauri-drag-region>
            <b>{query ? "Search" : current?.name ?? "Mail"}</b>
            <span>{query ? `${msgs.length} results` : current ? `${current.total} messages${current.unread ? `, ${current.unread} unread` : ""}` : ""}</span>
          </div>
          <button className="icon" onClick={() => setCompose({ mode: "new" })} title="New message (Ctrl+N)"><Icon name="compose" /></button>
          <button className="icon" onClick={syncNow} disabled={syncing || !online} title="Get new mail"><span className={syncing ? "spin" : ""}><Icon name="refresh" /></span></button>
        </div>
        <div className="search">
          <input placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
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
                      <span className="date">{m.has_attachments && <Icon name="paperclip" size={12} />}{m.is_flagged && <span style={{ color: "#ff9f0a" }}><Icon name="flag" size={12} /></span>}{fmtDate(m.received_at)}</span>
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
        <div className="toolbar" data-tauri-drag-region>
          <div className="title" data-tauri-drag-region />
          {selected && (
            <div className="actions">
              <button className="icon" onClick={() => startCompose("reply")} title="Reply (Ctrl+R)"><Icon name="reply" /></button>
              <button className="icon" onClick={() => startCompose("replyAll")} title="Reply all (Ctrl+Shift+R)"><Icon name="replyall" /></button>
              <button className="icon" onClick={() => startCompose("forward")} title="Forward (Ctrl+Shift+F)"><Icon name="forward" /></button>
              <span style={{ width: 8 }} />
              <button className="icon" onClick={() => toggleRead(selected)} title={selected.is_read ? "Mark as unread" : "Mark as read"}><Icon name="unread" /></button>
              <button className="icon" onClick={() => toggleFlag(selected)} title={selected.is_flagged ? "Unflag" : "Flag"} style={selected.is_flagged ? { color: "#ff9f0a" } : undefined}><Icon name="flag" /></button>
              <span style={{ position: "relative" }}>
                <button className="icon" onClick={() => setMoveOpen(!moveOpen)} title="Move to folder"><Icon name="move" /></button>
                {moveOpen && (
                  <div className="menu">
                    {folders.filter((f) => f.id !== folderId).map((f) => <div key={f.id} onClick={() => moveTo(selected, f)}>{f.name}</div>)}
                  </div>
                )}
              </span>
              <button className="icon" onClick={() => del(selected)} title="Delete (Del)"><Icon name="trash" /></button>
              <span style={{ width: 8 }} />
            </div>
          )}
          {selected && theme === "dark" && (
            <button className="icon" onClick={() => setDarkMail(!darkMail)} title={darkMail ? "Show email in light mode" : "Show email in dark mode"}><Icon name={darkMail ? "sun" : "moon"} /></button>
          )}
          <button className="icon" onClick={() => setTheme(theme === "dark" ? "light" : "dark")} title="Toggle appearance"><Icon name="appearance" /></button>
        </div>
        {selected ? (
          <>
            <header>
              <h2>{selected.subject || "(no subject)"}</h2>
              <div className="sender">
                <div className="avatar">{initials(selected.sender_name || selected.sender_address)}</div>
                <div className="meta">
                  <div><b>{selected.sender_name || selected.sender_address}</b> &lt;{selected.sender_address}&gt;</div>
                  {people(selected.to_json) && <div>To: {people(selected.to_json)}</div>}
                  {people(selected.cc_json) && <div>Cc: {people(selected.cc_json)}</div>}
                </div>
                <div className="when">{new Date(selected.received_at * 1000).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}</div>
              </div>
            </header>
            {rendered && rendered.blocked > 0 && !allowRemote && (
              <div className="notice">
                <span>Remote content was blocked to protect your privacy.</span>
                <button onClick={() => setAllowRemote(true)}>Load</button>
              </div>
            )}
            {rendered ? (
              <div className="bodywrap">
                <iframe sandbox="allow-same-origin" srcDoc={rendered.doc} title="message" />
              </div>
            ) : (
              <div className="empty">This message hasn't been downloaded for offline reading yet. It will appear after the next sync.</div>
            )}
          </>
        ) : (
          <div className="empty">No message selected</div>
        )}
      </section>
      </>}
      {compose && (
        <Compose init={compose} onClose={() => setCompose(null)}
          onSent={(queued) => { setCompose(null); flash(queued ? "You're offline – message will send when you reconnect" : "Message sent"); refreshQueue(); }} />
      )}
      {queueOpen && (
        <div className="sheet-backdrop" onClick={() => setQueueOpen(false)}>
          <div className="sheet evsheet" onClick={(e) => e.stopPropagation()}>
            <div className="sheet-bar"><span /><b>Changes waiting to sync</b><button onClick={() => setQueueOpen(false)}>Close</button></div>
            <div className="evbody">
              {queue.length === 0 && <div className="muted">Everything is synced.</div>}
              {queue.map((q) => (
                <div key={q.seq}>
                  <b>{q.what}</b> <span className="muted">· {q.failed ? "not accepted by the server" : q.tries ? `will retry (tried ${q.tries}×)` : "waiting"}</span>
                  {q.error && <div className="muted" style={{ fontSize: 12 }}>{q.error}</div>}
                </div>
              ))}
              {queue.length > 0 && !online && <div className="muted">You're offline. These will be sent when you reconnect.</div>}
            </div>
            <div className="sheet-bar">
              {rejected > 0 ? <button onClick={() => invoke("discard_failed").then(refreshQueue).catch((e) => flash(String(e)))}>Discard not accepted</button> : <span />}
              <button className="primary" onClick={async () => { try { await invoke("retry_pending"); flash("Sent"); } catch (e) { flash(String(e)); } refreshQueue(); loadFolders(); }}>Try again now</button>
            </div>
          </div>
        </div>
      )}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
