import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";

type Acct = { email: string; display_name: string; provider?: string; rules?: boolean };
type Fld = { id: string; name: string; well_known: string | null; parent_remote_id: string | null; remote_id: string; account_email: string };
type Rule = {
  id: string | null; account: string; name: string; enabled: boolean;
  from: string; subject: string; to: string; body: string;
  move_to: string | null; mark_read: boolean; flag: boolean; delete: boolean;
};
type RuleView = Rule & { folder_name: string | null; provider: string };

const blank = (account: string): Rule => ({ id: null, account, name: "", enabled: true, from: "", subject: "", to: "", body: "", move_to: null, mark_read: false, flag: false, delete: false });

function summary(r: RuleView) {
  const when = [r.from && `from “${r.from}”`, r.subject && `subject has “${r.subject}”`, r.to && `to/cc has “${r.to}”`, r.body && `text has “${r.body}”`].filter(Boolean).join(" and ");
  const flagWord = r.provider === "google" ? "star it" : "mark it important";
  const doWhat = r.delete ? "delete it" : [r.move_to && `move it to “${r.folder_name ?? "a folder"}”`, r.mark_read && "mark it read", r.flag && flagWord].filter(Boolean).join(", ");
  return `If ${when}: ${doWhat}.`;
}

export default function Rules({ accounts, folders, onClose, onToast, onAccountsChanged }: {
  accounts: Acct[]; folders: Fld[]; onClose: () => void; onToast: (t: string) => void; onAccountsChanged: () => void;
}) {
  const [email, setEmail] = useState(accounts[0]?.email ?? "");
  const [rules, setRules] = useState<RuleView[]>([]);
  const [edit, setEdit] = useState<Rule | null>(null);
  const [runFor, setRunFor] = useState<{ id: string; folder: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const acct = accounts.find((a) => a.email === email);
  const google = acct?.provider === "google";
  const mine = folders.filter((f) => f.account_email.toLowerCase() === email.toLowerCase());
  // folders as an indented list (children under their parent)
  const options = useMemo(() => {
    const ids = new Set(mine.map((f) => f.remote_id));
    const kids = new Map<string, Fld[]>();
    const roots: Fld[] = [];
    for (const f of mine) {
      if (f.parent_remote_id && ids.has(f.parent_remote_id)) { const k = kids.get(f.parent_remote_id) ?? []; k.push(f); kids.set(f.parent_remote_id, k); } else roots.push(f);
    }
    const out: { f: Fld; depth: number }[] = [];
    const walk = (l: Fld[], d: number) => l.forEach((f) => { out.push({ f, depth: d }); walk(kids.get(f.remote_id) ?? [], d + 1); });
    walk(roots, 0);
    return out.filter(({ f }) => !(google && (f.well_known === "sentitems" || f.well_known === "drafts")));
  }, [mine, google]);

  const load = () => invoke<RuleView[]>("list_rules").then(setRules).catch((e) => setError(String(e)));
  useEffect(() => { load(); }, []);

  async function run<T>(fn: () => Promise<T>) {
    setBusy(true); setError("");
    try { return await fn(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }

  const allow = () => run(async () => { await invoke("enable_rules", { email }); onToast("Rules are allowed for this account."); onAccountsChanged(); });
  const save = () => edit && run(async () => { await invoke("save_rule", { rule: edit }); setEdit(null); await load(); onToast("Rule saved"); });
  const remove = async (r: RuleView) => {
    if (!(await ask(`Delete the rule “${r.name}”?`, { title: "Delete rule", kind: "warning" }))) return;
    run(async () => { await invoke("delete_rule", { id: r.id }); await load(); });
  };
  const toggle = (r: RuleView) => run(async () => { await invoke("set_rule_enabled", { id: r.id, on: !r.enabled }); await load(); });
  const runNow = () => runFor && run(async () => {
    const n = await invoke<number>("run_rule", { id: runFor.id, folderId: runFor.folder });
    setRunFor(null);
    onToast(n ? `Rule applied to ${n} message${n === 1 ? "" : "s"}` : "No messages matched");
  });

  const inbox = (e: string) => folders.find((f) => f.account_email.toLowerCase() === e.toLowerCase() && f.well_known === "inbox")?.id ?? "";
  const mineRules = rules.filter((r) => r.account.toLowerCase() === email.toLowerCase());
  const set = (p: Partial<Rule>) => setEdit((r) => (r ? { ...r, ...p } : r));

  return (
    <div className="sheet-backdrop" onClick={() => !busy && onClose()}>
      <div className="sheet" style={{ height: "min(660px, 86vh)" }} onClick={(e) => e.stopPropagation()}>
        <div className="sheet-bar"><span /><b>Mail rules</b><button disabled={busy} onClick={onClose}>Close</button></div>
        <div className="evbody" style={{ flex: 1 }}>
          {accounts.length > 1 && (
            <label className="rulefield">Account
              <select value={email} onChange={(e) => { setEmail(e.target.value); setEdit(null); }}>
                {accounts.map((a) => <option key={a.email} value={a.email}>{a.display_name} ({a.email})</option>)}
              </select>
            </label>
          )}
          <div className="muted" style={{ fontSize: 12 }}>
            Rules are kept on {google ? "Gmail (as filters)" : "Outlook (as inbox rules)"}, so they also work when this app is closed. They apply to new mail as it arrives.
          </div>

          {acct && !acct.rules ? (
            <div className="notice" style={{ margin: 0 }}>
              <span>Rules need one extra permission for {acct.email}. You'll sign in once more in your browser.</span>
              <button className="primary" disabled={busy} onClick={allow}>{busy ? "Waiting for browser…" : "Allow rules"}</button>
            </div>
          ) : edit ? (
            <div className="form" style={{ padding: 0 }}>
              <b>{edit.id ? "Edit rule" : "New rule"}</b>
              <input placeholder="Rule name (optional)" value={edit.name} onChange={(e) => set({ name: e.target.value })} />
              <div className="muted" style={{ fontSize: 11, fontWeight: 600 }}>WHEN A MESSAGE ARRIVES AND…</div>
              <label className="rulefield">From contains<input placeholder="e.g. boss@company.com or Acme" value={edit.from} onChange={(e) => set({ from: e.target.value })} /></label>
              <label className="rulefield">Subject contains<input value={edit.subject} onChange={(e) => set({ subject: e.target.value })} /></label>
              <label className="rulefield">To / Cc contains<input value={edit.to} onChange={(e) => set({ to: e.target.value })} /></label>
              <label className="rulefield">Words in the message<input value={edit.body} onChange={(e) => set({ body: e.target.value })} /></label>
              <div className="muted" style={{ fontSize: 11, fontWeight: 600 }}>DO THIS</div>
              <label className="rulefield">Move to folder
                <select disabled={edit.delete} value={edit.move_to ?? ""} onChange={(e) => set({ move_to: e.target.value || null })}>
                  <option value="">— don't move —</option>
                  {options.map(({ f, depth }) => <option key={f.id} value={f.id}>{"  ".repeat(depth)}{f.name}</option>)}
                </select>
              </label>
              <label className="check"><input type="checkbox" disabled={edit.delete} checked={edit.mark_read} onChange={(e) => set({ mark_read: e.target.checked })} /> Mark as read</label>
              <label className="check"><input type="checkbox" disabled={edit.delete} checked={edit.flag} onChange={(e) => set({ flag: e.target.checked })} /> {google ? "Star it" : "Mark as important"}{!google && <span className="muted"> (Outlook rules can't set a flag)</span>}</label>
              <label className="check"><input type="checkbox" checked={edit.delete} onChange={(e) => set({ delete: e.target.checked, move_to: e.target.checked ? null : edit.move_to, mark_read: false, flag: false })} /> Delete it (moves to Trash)</label>
              <div className="prompt-actions">
                <button disabled={busy} onClick={() => { setEdit(null); setError(""); }}>Cancel</button>
                <button className="primary" disabled={busy} onClick={save}>{busy ? "Saving…" : "Save rule"}</button>
              </div>
            </div>
          ) : (
            <>
              <div><button className="primary" onClick={() => { setError(""); setEdit(blank(email)); }}>New rule</button></div>
              {mineRules.length === 0 && <div className="muted">No rules yet. A rule could, for example, move everything from your bank into a “Bank” folder.</div>}
              {mineRules.map((r) => (
                <div key={r.id} className="rulecard">
                  <div className="rulehead">
                    <b>{r.name}</b>
                    <input type="checkbox" className="switch" checked={r.enabled} disabled={busy} onChange={() => toggle(r)} title={r.enabled ? "On" : "Off"} />
                  </div>
                  <div className="muted" style={{ fontSize: 12.5 }}>{summary(r)}</div>
                  <div className="rowbtns" style={{ marginTop: 6 }}>
                    <button disabled={busy} onClick={() => { setError(""); setEdit({ ...r }); }}>Edit</button>
                    <button disabled={busy} onClick={() => setRunFor({ id: r.id!, folder: inbox(email) })}>Run on existing mail…</button>
                    <button disabled={busy} onClick={() => remove(r)}>Delete</button>
                  </div>
                  {runFor?.id === r.id && (
                    <div className="rowbtns" style={{ marginTop: 8, alignItems: "center" }}>
                      <span className="muted">Apply to mail in</span>
                      <select value={runFor.folder} onChange={(e) => setRunFor({ ...runFor, folder: e.target.value })}>
                        {options.map(({ f, depth }) => <option key={f.id} value={f.id}>{"  ".repeat(depth)}{f.name}</option>)}
                      </select>
                      <button className="primary" disabled={busy} onClick={runNow}>Run</button>
                      <button onClick={() => setRunFor(null)}>Cancel</button>
                    </div>
                  )}
                </div>
              ))}
              {google && <div className="muted" style={{ fontSize: 11.5 }}>Gmail filters have no off switch: a rule you switch off is removed from Gmail and put back when you switch it on.</div>}
            </>
          )}
          {error && <div className="error">{error}</div>}
        </div>
      </div>
    </div>
  );
}
