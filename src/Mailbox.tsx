import React, { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import DOMPurify from "dompurify";
import { DragBar } from "./WindowControls";
import Icon from "./Icon";
import Compose, { ComposeInit } from "./Compose";
import CalendarView, { Cal } from "./CalendarView";
import Settings from "./Settings";
import Rules from "./Rules";
import { Ribbon, RibbonTab, Group, Big, Small, Stack, BigMenu, NamePrompt } from "./Ribbon";
import { ask, save as saveDialog } from "@tauri-apps/plugin-dialog";

type Account = { email: string; display_name: string; provider?: string; rules?: boolean };
const ALL = "all-inboxes";
const PALETTE = ["#0a84ff", "#30d158", "#ff9f0a", "#bf5af2", "#ff375f", "#64d2ff"];
type Folder = { id: string; remote_id: string; name: string; well_known: string | null; unread: number; total: number; parent_remote_id: string | null; account_email: string };
type Msg = {
  id: string; subject: string | null; sender_name: string | null; sender_address: string | null;
  snippet: string | null; received_at: number; is_read: boolean; is_flagged: boolean; has_attachments: boolean; account_email: string;
};
type Att = { id: string; name: string; mime: string; size: number; downloaded: boolean };
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

const fmtBytes = (b: number) => (b >= 1 << 20 ? `${(b / (1 << 20)).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`);

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

/** Folders of one account as a tree: children under their parent, indented. */
function buildTree(folders: Folder[]) {
  const ids = new Set(folders.map((f) => f.remote_id));
  const kids = new Map<string, Folder[]>();
  const roots: Folder[] = [];
  for (const f of folders) {
    if (f.parent_remote_id && ids.has(f.parent_remote_id)) {
      const k = kids.get(f.parent_remote_id) ?? []; k.push(f); kids.set(f.parent_remote_id, k);
    } else roots.push(f);
  }
  const out: { f: Folder; depth: number }[] = [];
  const walk = (list: Folder[], depth: number) => { for (const f of list) { out.push({ f, depth }); walk(kids.get(f.remote_id) ?? [], depth + 1); } };
  walk(roots, 0);
  return out;
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

export default function Mailbox({ account: _first, onSignOut }: { account: Account; onSignOut: () => void }) {
  const [folders, setFolders] = useState<Folder[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => { try { return new Set(JSON.parse(localStorage.getItem("collapsedAccts") || "[]")); } catch { return new Set(); } });
  useEffect(() => { try { localStorage.setItem("collapsedAccts", JSON.stringify([...collapsed])); } catch { /* ignore */ } }, [collapsed]);
  const colorOf = (email: string) => PALETTE[Math.max(0, accounts.findIndex((a) => a.email.toLowerCase() === email.toLowerCase())) % PALETTE.length];
  const multi = accounts.length > 1;
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [atts, setAtts] = useState<Att[]>([]);
  const [attBusy, setAttBusy] = useState("");
  const [rulesOpen, setRulesOpen] = useState(false);
  const [tab, setTab] = useState<RibbonTab>("home");
  const [meetingReq, setMeetingReq] = useState({ teams: true, n: 0 });
  const [prompt, setPrompt] = useState<null | { kind: "new" | "sub" | "rename" }>(null);
  const pending = queue.filter((q) => !q.failed).length;
  const rejected = queue.filter((q) => q.failed).length;
  const refreshQueue = () => invoke<QueueItem[]>("pending_details").then(setQueue).catch(() => {});
  const [, setMoveOpen] = useState(false);
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
    const [f, accs] = await Promise.all([invoke<Folder[]>("list_folders"), invoke<Account[]>("list_accounts")]);
    setFolders(f);
    setAccounts(accs);
    const firstInbox = f.find((x) => x.well_known === "inbox")?.id ?? f[0]?.id ?? null;
    setFolderId((cur) => {
      if (cur === ALL) return accs.length > 1 ? ALL : firstInbox;
      if (cur && f.some((x) => x.id === cur)) return cur;
      return accs.length > 1 ? ALL : firstInbox;
    });
    return accs;
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
  /** The account a brand-new message should come from: the folder you're in, else the open message's account. */
  const newMail = () => setCompose({ mode: "new", from: folders.find((f) => f.id === folderId)?.account_email ?? selected?.account_email ?? accounts[0]?.email });
  const startCompose = (mode: ComposeInit["mode"]) => {
    if (mode === "new" || !selected) return newMail();
    const s = selected;
    const me = s.account_email.toLowerCase();
    const header = `On ${new Date(s.received_at * 1000).toLocaleString()}, ${s.sender_name ?? ""} &lt;${s.sender_address ?? ""}&gt; wrote:`;
    const subj = s.subject ?? "";
    const base = { replyTo: s.id, from: s.account_email, quoteHtml: s.body_html ?? "", quoteHeader: header };
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
      else if (!selected) { if ((e.ctrlKey || e.metaKey) && e.key === "n") newMail(); }
      else if (e.key === "Delete") del(selected);
      else if ((e.ctrlKey || e.metaKey) && e.key === "n") newMail();
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
    loadFolders().then(() => syncNow());
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
    setAtts([]);
    if (m.has_attachments) invoke<Att[]>("list_attachments", { messageId: m.id }).then((a) => setSelected((s) => { if (s && s.id === m.id) setAtts(a); return s; })).catch(() => {});
    if (!m.is_read) {
      invoke("set_read", { id: m.id, read: true }).then(() => { patch(m.id, { is_read: true }); loadFolders(); }).catch(() => {});
    }
  };

  const rendered = useMemo(
    () => (selected?.body_html ? buildDoc(selected.body_html, allowRemote, theme === "dark" && darkMail) : null),
    [selected, allowRemote, theme, darkMail],
  );

  const current = folders.find((f) => f.id === folderId);
  const inboxes = folders.filter((f) => f.well_known === "inbox");
  const allUnread = inboxes.reduce((n, f) => n + f.unread, 0);
  const allTotal = inboxes.reduce((n, f) => n + f.total, 0);
  const groups = useMemo(
    () => accounts.map((acc) => ({ acc, tree: buildTree(folders.filter((f) => f.account_email.toLowerCase() === acc.email.toLowerCase())) })),
    [accounts, folders],
  );
  const providerOf = (email?: string) => accounts.find((a) => a.email.toLowerCase() === email?.toLowerCase())?.provider ?? "microsoft";

  const builtIn = !!current?.well_known;
  const folderAction = async (name: string, forAccount?: string) => {
    const kind = prompt?.kind;
    setPrompt(null);
    try {
      if (kind === "new") { const id = await invoke<string>("create_folder", { name, parentId: null, account: forAccount ?? null }); flash(`Folder "${name}" created`); await loadFolders(); setFolderId(id); }
      else if (kind === "sub" && current) { const id = await invoke<string>("create_folder", { name, parentId: current.id }); flash(`Folder "${name}" created inside ${current.name}`); await loadFolders(); setFolderId(id); }
      else if (kind === "rename" && current) { await invoke("rename_folder", { id: current.id, name }); flash("Folder renamed"); loadFolders(); }
    } catch (e) { flash(String(e)); }
  };
  const deleteFolder = async () => {
    if (!current || builtIn) return;
    const gmail = providerOf(current.account_email) === "google";
    if (!(await ask(gmail ? `Delete the label "${current.name}"? Its messages stay in your Gmail account (in All Mail).` : `Delete the folder "${current.name}" and everything in it? Microsoft 365 moves it to Deleted Items.`, { title: "Delete folder", kind: "warning" }))) return;
    try {
      await invoke("delete_folder", { id: current.id });
      flash("Folder deleted");
      setFolderId(folders.find((f) => f.well_known === "inbox" && f.account_email === current.account_email)?.id ?? null);
      setSelected(null);
      loadFolders();
    } catch (e) { flash(String(e)); }
  };
  const archiveFolder = folders.find((f) => f.well_known === "archive" && f.account_email === selected?.account_email);
  const newMeeting = (teams: boolean) => { setView("calendar"); setMeetingReq((r) => ({ teams, n: r.n + 1 })); };
  // A message can only be moved within its own account.
  const moveItems = (selected ? buildTree(folders.filter((f) => f.account_email === selected.account_email)) : [])
    .filter(({ f }) => f.id !== folderId && !(providerOf(f.account_email) === "google" && (f.well_known === "sentitems" || f.well_known === "drafts")))
    .map(({ f, depth }) => ({ label: f.name, indent: depth, onClick: () => selected && moveTo(selected, f) }));
  const mailTab = view === "mail";
  const initials = (n?: string | null) => (n || "?").split(/\s+/).map((w) => w[0]).slice(0, 2).join("").toUpperCase();

  return (
    <div className="shell">
      <nav className="sidebar">
        <DragBar><img src="/logo.png" alt="" className="brand-logo" /><span className="brand-name">Graphite Mail</span></DragBar>
        <div className={`folder navitem ${view === "calendar" ? "active" : ""}`} onClick={() => setView("calendar")}>
          <span className="ico"><Icon name="calendar" /></span><span className="name" title="All your accounts' calendars together">Master Calendar</span>
        </div>
        <div className="folders">
          {view === "calendar" && cals.length > 0 && <div className="section" style={{ padding: "6px 10px 4px" }}>My calendars <span style={{ fontWeight: 400 }}>· tick to show</span></div>}
          {view === "calendar" && cals.map((c, i) => (
            <React.Fragment key={c.id}>
            {multi && (i === 0 || cals[i - 1].account_email !== c.account_email) && <div className="muted acct-sub">{accounts.find((a) => a.email.toLowerCase() === c.account_email.toLowerCase())?.display_name ?? c.account_email}</div>}
            <div className="folder calrow" onClick={() => setView("calendar")} title="Open calendar">
              <input type="checkbox" checked={!hiddenCals.has(c.id)} style={{ accentColor: c.color }} title="Show or hide this calendar"
                onClick={(e) => e.stopPropagation()}
                onChange={() => setHiddenCals((h) => { const n = new Set(h); n.has(c.id) ? n.delete(c.id) : n.add(c.id); return n; })} />
              <span className="name">{c.name}</span>
            </div>
            </React.Fragment>
          ))}
          <div className="section" style={{ padding: "14px 10px 4px" }}>Mailboxes</div>
          {multi && (
            <div className={`folder ${folderId === ALL && !query && view === "mail" ? "active" : ""}`} onClick={() => { setView("mail"); setQuery(""); setFolderId(ALL); setSelected(null); }}>
              <span className="ico"><Icon name="inbox" /></span>
              <span className="name">All Inboxes</span>
              {allUnread > 0 && <span className="count">{allUnread}</span>}
            </div>
          )}
          {groups.map(({ acc, tree }) => {
            const shut = collapsed.has(acc.email);
            return (
              <div key={acc.email} className="acct">
                <div className="acct-head" onClick={() => setCollapsed((c) => { const n = new Set(c); n.has(acc.email) ? n.delete(acc.email) : n.add(acc.email); return n; })} title={shut ? "Show folders" : "Hide folders"}>
                  <span className="chev">{shut ? "▸" : "▾"}</span>
                  <i className="acctdot" style={{ background: colorOf(acc.email) }} />
                  <span className="acct-name"><b>{acc.display_name}</b><span className="muted">{acc.email}</span></span>
                </div>
                {!shut && tree.map(({ f, depth }) => (
                  <div key={f.id} style={{ paddingLeft: 10 + depth * 16 }} className={`folder ${f.id === folderId && !query && view === "mail" ? "active" : ""}`} onClick={() => { setView("mail"); setQuery(""); setFolderId(f.id); setSelected(null); }}>
                    <span className="ico"><Icon name={f.well_known ?? "folder"} /></span>
                    <span className="name">{f.name}</span>
                    {f.unread > 0 && <span className="count">{f.unread}</span>}
                  </div>
                ))}
              </div>
            );
          })}
          {!folders.length && <div className="empty">No folders yet</div>}
        </div>
        <div className="account">
          <div className="avatar">{multi ? String(accounts.length) : initials(accounts[0]?.display_name ?? _first.display_name)}</div>
          <div className="who">
            <div>{multi ? `${accounts.length} accounts` : accounts[0]?.display_name ?? _first.display_name}</div>
            <div className="muted" title={status}><span className={`dot ${online ? "" : "off"}`} />{online ? status || "Online" : "Offline – saved mail"}</div>
            {queue.length > 0 && (
              <div className="muted queue-link" onClick={() => { refreshQueue(); setQueueOpen(true); }} title="See what is waiting">
                {pending > 0 ? `${pending} waiting to sync` : ""}{pending > 0 && rejected > 0 ? " · " : ""}{rejected > 0 ? `${rejected} not accepted` : ""}
              </div>
            )}
          </div>
          <button className="icon" onClick={() => setSettingsOpen(true)} title="Settings"><Icon name="settings" /></button>
          <button className="icon" onClick={async () => { if (await ask(multi ? "Sign out of all accounts? Their downloaded mail is removed from this computer (nothing is deleted on the servers)." : "Sign out? Your downloaded mail is removed from this computer (nothing is deleted on the server).", { title: "Sign out", kind: "warning" })) onSignOut(); }} title={multi ? "Sign out of all accounts" : "Sign out"}><Icon name="signout" /></button>
        </div>
      </nav>

      <Ribbon tabs={[{ id: "home", label: "Home" }, { id: "folder", label: "Folder" }, { id: "view", label: "View" }]} active={tab} onTab={setTab}>
        {tab === "home" && <>
          <Group label="New">
            <Big icon="compose" label="New Email" accent onClick={() => newMail()} title="New email (Ctrl+N)" />
            <Big icon="meeting" label="New Meeting" onClick={() => newMeeting(true)} title="New Teams meeting" />
            <Stack><Small icon="calendar" label="New Event" onClick={() => newMeeting(false)} /></Stack>
          </Group>
          <Group label="Delete">
            <Big icon="trash" label="Delete" disabled={!mailTab || !selected} onClick={() => selected && del(selected)} title="Delete (Del)" />
            <Big icon="archive" label="Archive" disabled={!mailTab || !selected || !archiveFolder || folderId === archiveFolder.id} onClick={() => selected && archiveFolder && moveTo(selected, archiveFolder)} />
          </Group>
          <Group label="Respond">
            <Big icon="reply" label="Reply" disabled={!mailTab || !selected} onClick={() => startCompose("reply")} title="Reply (Ctrl+R)" />
            <Big icon="replyall" label="Reply All" disabled={!mailTab || !selected} onClick={() => startCompose("replyAll")} title="Reply all (Ctrl+Shift+R)" />
            <Big icon="forward" label="Forward" disabled={!mailTab || !selected} onClick={() => startCompose("forward")} title="Forward (Ctrl+Shift+F)" />
          </Group>
          <Group label="Move">
            <BigMenu icon="move" label="Move" disabled={!mailTab || !selected} items={moveItems} />
          </Group>
          <Group label="Tags">
            <Stack>
              <Small icon={selected?.is_read ? "unread" : "read"} label={selected?.is_read ? "Mark Unread" : "Mark Read"} disabled={!mailTab || !selected} onClick={() => selected && toggleRead(selected)} />
              <Small icon="flag" label={selected?.is_flagged ? "Clear Flag" : "Flag"} on={!!selected?.is_flagged} disabled={!mailTab || !selected} onClick={() => selected && toggleFlag(selected)} />
            </Stack>
          </Group>
          <Group label="Rules">
            <Big icon="rules" label="Rules" onClick={() => setRulesOpen(true)} title="Automatically move, mark or delete incoming mail" />
          </Group>
          <Group label="Sync">
            <Big icon="sync" label={syncing ? "Syncing…" : "Send / Receive"} disabled={syncing || !online} onClick={syncNow} />
          </Group>
        </>}
        {tab === "folder" && <>
          <Group label="New">
            <Big icon="newfolder" label="New Folder" onClick={() => setPrompt({ kind: "new" })} />
            <Big icon="subfolder" label="New Subfolder" disabled={!current || view !== "mail"} onClick={() => setPrompt({ kind: "sub" })} title={current ? `New folder inside ${current.name}` : undefined} />
          </Group>
          <Group label={current ? `“${current.name}”` : "Folder"}>
            <Big icon="rename" label="Rename" disabled={!current || builtIn || view !== "mail"} onClick={() => setPrompt({ kind: "rename" })} title={builtIn ? "Built-in folders can't be renamed" : undefined} />
            <Big icon="deletefolder" label="Delete Folder" disabled={!current || builtIn || view !== "mail"} onClick={deleteFolder} title={builtIn ? "Built-in folders can't be deleted" : undefined} />
          </Group>
          <Group label="Sync">
            <Big icon="sync" label="Refresh Folders" disabled={syncing || !online} onClick={syncNow} />
          </Group>
        </>}
        {tab === "view" && <>
          <Group label="Show">
            <Big icon="mail" label="Mail" onClick={() => setView("mail")} accent={view === "mail"} />
            <Big icon="calendar" label="Calendar" onClick={() => setView("calendar")} accent={view === "calendar"} />
          </Group>
          <Group label="Appearance">
            <Stack>
              <Small icon="sun" label="Light" on={theme === "light"} onClick={() => setTheme("light")} />
              <Small icon="moon" label="Dark" on={theme === "dark"} onClick={() => setTheme("dark")} />
            </Stack>
            <Stack>
              <Small icon={darkMail ? "moon" : "sun"} label={darkMail ? "Emails: dark" : "Emails: original"} disabled={theme !== "dark"} onClick={() => setDarkMail(!darkMail)} title="How email content looks in dark mode" />
            </Stack>
          </Group>
          <Group label="App">
            <Big icon="settings" label="Settings" onClick={() => setSettingsOpen(true)} />
          </Group>
        </>}
      </Ribbon>

      {view === "calendar" && <PaneBoundary><CalendarView calendars={cals} hidden={hiddenCals} onToast={flash} newRequest={meetingReq} /></PaneBoundary>}
      {view === "mail" && <>
      <section className="list">
        <div className="toolbar" data-tauri-drag-region>
          <div className="title" data-tauri-drag-region>
            <b>{query ? "Search" : folderId === ALL ? "All Inboxes" : current?.name ?? "Mail"}</b>
            <span>{query ? `${msgs.length} results` : folderId === ALL ? `${allTotal} messages${allUnread ? `, ${allUnread} unread` : ""}` : current ? `${current.total} messages${current.unread ? `, ${current.unread} unread` : ""}${multi ? ` · ${current.account_email}` : ""}` : ""}</span>
          </div>
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
                      <span className="from">{multi && (folderId === ALL || query) && <i className="acctdot" style={{ background: colorOf(m.account_email) }} title={m.account_email} />}{m.sender_name || m.sender_address || "(no sender)"}</span>
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
                  {multi && <div><i className="acctdot" style={{ background: colorOf(selected.account_email) }} />Received in {selected.account_email}</div>}
                </div>
                <div className="when">{new Date(selected.received_at * 1000).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}</div>
              </div>
            </header>
            {atts.length > 0 && (
              <div className="attrow">
                {atts.map((a) => (
                  <span key={a.id} className="attchip" title={`${a.name} · ${fmtBytes(a.size)}${a.downloaded ? " · saved for offline" : ""}`}>
                    <button className="attopen" disabled={attBusy === a.id}
                      onClick={async () => { setAttBusy(a.id); try { await invoke("open_attachment", { id: a.id }); setAtts((l) => l.map((x) => (x.id === a.id ? { ...x, downloaded: true } : x))); } catch (e) { flash(String(e)); } finally { setAttBusy(""); } }}>
                      <Icon name="paperclip" size={13} /> <span className="attname">{a.name}</span> <span className="muted">{attBusy === a.id ? "downloading…" : fmtBytes(a.size)}</span>
                    </button>
                    <button className="attsave" title="Save a copy…" disabled={attBusy === a.id}
                      onClick={async () => { const dest = await saveDialog({ defaultPath: a.name, title: "Save attachment" }); if (!dest) return; setAttBusy(a.id); try { await invoke("save_attachment", { id: a.id, dest }); flash("Saved"); } catch (e) { flash(String(e)); } finally { setAttBusy(""); } }}>Save</button>
                  </span>
                ))}
              </div>
            )}
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
        <Compose init={compose} accounts={accounts} onClose={() => setCompose(null)}
          onSent={(queued) => { setCompose(null); flash(queued ? "You're offline – message will send when you reconnect" : "Message sent"); refreshQueue(); }} />
      )}
      {rulesOpen && <Rules accounts={accounts} folders={folders} onClose={() => setRulesOpen(false)} onToast={flash} onAccountsChanged={() => { loadFolders(); }} />}
      {settingsOpen && <Settings accounts={accounts} onClose={() => setSettingsOpen(false)}
        onAccountsChanged={async () => {
          const accs = await loadFolders();
          if (!accs.length) { setSettingsOpen(false); onSignOut(); } else syncNow();
        }} />}
      {prompt && (
        <NamePrompt
          title={prompt.kind === "rename" ? `Rename “${current?.name}”` : prompt.kind === "sub" ? `New folder inside “${current?.name}”` : "New folder"}
          initial={prompt.kind === "rename" ? current?.name : ""}
          confirm={prompt.kind === "rename" ? "Rename" : "Create"}
          accounts={prompt.kind === "new" && multi ? accounts : undefined}
          defaultAccount={current?.account_email ?? selected?.account_email ?? accounts[0]?.email}
          onCancel={() => setPrompt(null)} onSubmit={folderAction} />
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
