import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Icon from "./Icon";
import { Acct } from "./accounts";
import { initials } from "./recipients";

export type ContactRow = { id: string; source: "directory" | "personal"; account_email: string; name: string; email: string | null; subtitle: string | null };
type Group = { total: number; rows: ContactRow[] };
export type ContactStatus = {
  email: string; granted: boolean;
  /** "ok" | "never" | "needs_consent" | "blocked" | "error" | "removed" */
  directory: string; personal: string;
  directory_count: number; personal_count: number; last_sync: number | null;
};
export type Overview = { directory: Group; personal: Group; accounts: ContactStatus[] };
export type ContactDetail = {
  id: string; source: "directory" | "personal"; account_email: string; folder: string | null; name: string;
  emails: string[]; phones: { kind: string; number: string }[];
  job_title: string | null; department: string | null; office: string | null; company: string | null;
};

const GROUPS = [
  { key: "directory", label: "Directory", hint: "Your organisation's address list" },
  { key: "personal", label: "My contacts", hint: "Your own contacts" },
] as const;
type GroupKey = (typeof GROUPS)[number]["key"];

/** Why one account's contacts can't be shown, in words (null when nothing needs saying). */
export function statusNote(s: ContactStatus): { text: string; grant: boolean } | null {
  const parts = [s.directory, s.personal];
  if (parts.includes("needs_consent")) return { text: `Graphite Mail needs your permission to read contacts and your organisation's directory for ${s.email}.`, grant: true };
  if (parts.includes("blocked")) {
    const what = s.directory === "blocked" && s.personal === "blocked" ? "contacts and the directory" : s.directory === "blocked" ? "the directory" : "your contacts";
    return { text: `${s.email}: your organisation doesn't allow Graphite Mail to read ${what}. An administrator can allow it; then use Grant access again.`, grant: true };
  }
  if (parts.includes("removed")) return { text: `Cached contacts of ${s.email} were removed. Use Refresh contacts to download them again.`, grant: false };
  if (parts.includes("error")) return { text: `Contacts of ${s.email} couldn't be updated just now (offline?). Showing what is saved on this computer.`, grant: false };
  return null;
}

async function copy(text: string) {
  try { await navigator.clipboard.writeText(text); return true; } catch {
    const t = document.createElement("textarea");
    t.value = text; document.body.appendChild(t); t.select();
    const ok = document.execCommand("copy"); t.remove(); return ok;
  }
}

/** Contacts of the Microsoft 365 accounts: the organisation's directory and the user's own contacts, from the local copy. */
export default function ContactsView({ accounts, onNewEmail, onToast }: {
  accounts: Acct[]; onNewEmail: (to: string, from: string) => void; onToast: (t: string) => void;
}) {
  const ms = accounts.filter((a) => (a.provider ?? "microsoft") === "microsoft");
  const [query, setQuery] = useState("");
  const [account, setAccount] = useState("");
  const [data, setData] = useState<Overview | null>(null);
  const [shut, setShut] = useState<Record<GroupKey, boolean>>({ directory: false, personal: false });
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<ContactDetail | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [granting, setGranting] = useState("");
  const [error, setError] = useState("");
  const asked = useRef(0);
  const firstLoad = useRef(true);

  const load = () => {
    const mine = ++asked.current;
    return invoke<Overview>("contacts_overview", { query, account: account || null })
      .then((o) => { if (mine === asked.current) setData(o); return o; })
      .catch((e) => { setError(String(e)); return null; });
  };

  async function refresh(quiet = false) {
    setRefreshing(true); setError("");
    try { await invoke("refresh_contacts", { account: account || null }); await load(); if (!quiet) onToast("Contacts updated"); }
    catch (e) { setError(String(e)); }
    finally { setRefreshing(false); }
  }

  useEffect(() => {
    if (!ms.length) return;
    const t = setTimeout(async () => {
      const o = await load();
      // The first visit downloads at once instead of waiting for the next background pass.
      if (firstLoad.current && o && o.accounts.some((s) => s.directory === "never" || s.personal === "never")) refresh(true);
      firstLoad.current = false;
    }, firstLoad.current ? 0 : 150);
    return () => clearTimeout(t);
  }, [query, account, ms.length]);

  useEffect(() => {
    if (!selected) { setDetail(null); return; }
    invoke<ContactDetail>("contact_detail", { id: selected }).then(setDetail).catch((e) => { setDetail(null); setError(String(e)); });
  }, [selected]);

  async function grant(email: string) {
    setGranting(email); setError("");
    try { await invoke("grant_contacts", { email }); await load(); onToast("Access granted"); }
    catch (e) { setError(String(e)); }
    finally { setGranting(""); }
  }

  if (!ms.length) {
    return (
      <section className="contacts-only">
        <div className="empty"><Icon name="contacts" size={28} /><p>Contacts are available for Microsoft 365 accounts.</p></div>
      </section>
    );
  }

  const notes = (data?.accounts ?? []).map((s) => ({ s, note: statusNote(s) })).filter((x) => x.note);
  const total = (data?.directory.total ?? 0) + (data?.personal.total ?? 0);

  return (
    <>
      <section className="list contacts-list" aria-label="Contacts">
        <div className="toolbar" data-tauri-drag-region>
          <div className="title" data-tauri-drag-region>
            <b>Contacts</b>
            <span>{data ? `${total} ${query ? "matches" : "people"}` : "Loading…"}</span>
          </div>
          <button className="icon" onClick={() => refresh()} disabled={refreshing} title="Refresh contacts" aria-label="Refresh contacts">
            <span className={refreshing ? "spin" : ""}><Icon name="refresh" /></span>
          </button>
        </div>
        <div className="search">
          <input type="search" placeholder="Search contacts" aria-label="Search contacts" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        {ms.length > 1 && (
          <div className="search">
            <select aria-label="Account" value={account} onChange={(e) => { setAccount(e.target.value); setSelected(null); }} style={{ width: "100%" }}>
              <option value="">All Microsoft 365 accounts</option>
              {ms.map((a) => <option key={a.email} value={a.email}>{a.email}</option>)}
            </select>
          </div>
        )}
        <div className="scroller">
          {notes.map(({ s, note }) => (
            <div key={s.email} className="notice cnotice" role="status">
              <span>{note!.text}</span>
              {note!.grant && <button className="primary" disabled={!!granting} onClick={() => grant(s.email)}>{granting === s.email ? "Waiting for browser…" : "Grant access"}</button>}
            </div>
          ))}
          {error && <div className="error" role="alert" style={{ padding: "4px 8px" }}>{error}</div>}
          {GROUPS.map((g) => {
            const grp = data?.[g.key];
            const open = !shut[g.key];
            return (
              <div key={g.key} className="cgroup" role="group" aria-label={g.label}>
                <button className="cgroup-head" aria-expanded={open} title={g.hint} onClick={() => setShut((s) => ({ ...s, [g.key]: !s[g.key] }))}>
                  <span className="chev">{open ? "▾" : "▸"}</span><span className="cgroup-name">{g.label}</span><span className="count">{grp?.total ?? 0}</span>
                </button>
                {open && (
                  <div role="listbox" aria-label={`${g.label} list`}>
                    {grp?.rows.map((r) => (
                      <div key={r.id} role="option" aria-selected={selected === r.id} className={`crow ${selected === r.id ? "active" : ""}`} onClick={() => setSelected(r.id)}>
                        <div className="avatar">{initials(r.name)}</div>
                        <div className="crow-text">
                          <div className="cname">{r.name}</div>
                          <div className="csub">{r.subtitle ?? r.email ?? ""}</div>
                        </div>
                      </div>
                    ))}
                    {grp && grp.total === 0 && <div className="muted cnone">{query ? "No matches" : "No contacts here yet"}</div>}
                    {grp && grp.total > grp.rows.length && <div className="muted cnone">Showing {grp.rows.length} of {grp.total}. Search to find the others.</div>}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </section>

      <section className="reader contact-detail" aria-label="Contact details">
        {detail ? (
          <div className="cdetail">
            <header className="cdetail-head">
              <div className="avatar big">{initials(detail.name)}</div>
              <div>
                <h2>{detail.name}</h2>
                {(detail.job_title || detail.department) && <div className="muted">{[detail.job_title, detail.department].filter(Boolean).join(" · ")}</div>}
              </div>
            </header>
            <div className="rowbtns">
              <button className="primary" disabled={!detail.emails.length} onClick={() => onNewEmail(detail.emails[0], detail.account_email)}><Icon name="compose" /> New email</button>
              <button disabled={!detail.emails.length} onClick={async () => onToast((await copy(detail.emails[0])) ? "Email address copied" : "Couldn't copy")}><Icon name="copy" /> Copy email</button>
            </div>
            <dl className="cfields">
              {detail.emails.length > 0 && <><dt>Email</dt><dd>{detail.emails.map((e) => <div key={e}>{e}</div>)}</dd></>}
              {detail.phones.length > 0 && <><dt>Phone</dt><dd>{detail.phones.map((p) => <div key={p.number}>{p.number} <span className="muted">({p.kind})</span></div>)}</dd></>}
              {detail.job_title && <><dt>Job title</dt><dd>{detail.job_title}</dd></>}
              {detail.department && <><dt>Department</dt><dd>{detail.department}</dd></>}
              {detail.office && <><dt>Office</dt><dd>{detail.office}</dd></>}
              {detail.company && <><dt>Company</dt><dd>{detail.company}</dd></>}
            </dl>
            <div className="muted csource">
              {detail.source === "directory" ? "Directory" : `My contacts${detail.folder ? ` · ${detail.folder}` : ""}`}{ms.length > 1 ? ` · ${detail.account_email}` : ""}
            </div>
          </div>
        ) : (
          <div className="empty">No contact selected</div>
        )}
      </section>
    </>
  );
}
