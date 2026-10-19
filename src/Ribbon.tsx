import { ReactNode, useState } from "react";
import Icon from "./Icon";
import { TrafficLights } from "./WindowControls";

export type RibbonTab = "home" | "folder" | "view";

/** Outlook-style ribbon: tab strip on top, grouped labelled buttons below. */
export function Ribbon({ tabs, children, active, onTab }: {
  tabs: { id: RibbonTab; label: string }[];
  active: RibbonTab;
  onTab: (t: RibbonTab) => void;
  children: ReactNode;
}) {
  return (
    <header className="ribbon">
      <div className="ribbon-tabs" data-tauri-drag-region>
        {tabs.map((t) => (
          <button key={t.id} className={`rtab ${active === t.id ? "on" : ""}`} onClick={() => onTab(t.id)}>{t.label}</button>
        ))}
        <div className="ribbon-spacer" data-tauri-drag-region />
        <TrafficLights />
      </div>
      <div className="ribbon-body">{children}</div>
    </header>
  );
}

export function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="rgroup">
      <div className="rgroup-items">{children}</div>
      <div className="rgroup-label">{label}</div>
    </div>
  );
}

/** Large button: icon above label. */
export function Big({ icon, label, onClick, disabled, title, accent }: { icon: string; label: string; onClick: () => void; disabled?: boolean; title?: string; accent?: boolean }) {
  return (
    <button className={`rbig ${accent ? "accent" : ""}`} onClick={onClick} disabled={disabled} title={title ?? label}>
      <span className="rbig-ico"><Icon name={icon} size={22} /></span>
      <span>{label}</span>
    </button>
  );
}

/** Small button: icon beside label, stacked in a column. */
export function Small({ icon, label, onClick, disabled, title, on }: { icon: string; label: string; onClick: () => void; disabled?: boolean; title?: string; on?: boolean }) {
  return (
    <button className={`rsmall ${on ? "on" : ""}`} onClick={onClick} disabled={disabled} title={title ?? label}>
      <Icon name={icon} size={15} /><span>{label}</span>
    </button>
  );
}

export function Stack({ children }: { children: ReactNode }) {
  return <div className="rstack">{children}</div>;
}

/** Big button with a drop-down menu of choices. */
export function BigMenu({ icon, label, disabled, items }: { icon: string; label: string; disabled?: boolean; items: { label: string; onClick: () => void; indent?: number }[] }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="rmenu-wrap" onMouseLeave={() => setOpen(false)}>
      <button className="rbig" disabled={disabled} onClick={() => setOpen(!open)} title={label}>
        <span className="rbig-ico"><Icon name={icon} size={22} /></span>
        <span>{label} ▾</span>
      </button>
      {open && !disabled && (
        <div className="menu rmenu">
          {items.length === 0 && <div className="muted">No folders</div>}
          {items.map((it, i) => (
            <div key={i} style={{ paddingLeft: 10 + (it.indent ?? 0) * 14 }} onClick={() => { setOpen(false); it.onClick(); }}>{it.label}</div>
          ))}
        </div>
      )}
    </span>
  );
}

/** Small sheet asking for a single text value (folder names etc.), optionally with an account to put it in. */
export function NamePrompt({ title, initial, confirm, onCancel, onSubmit, accounts, defaultAccount }: {
  title: string; initial?: string; confirm: string; onCancel: () => void; onSubmit: (v: string, account?: string) => void;
  accounts?: { email: string; display_name: string }[]; defaultAccount?: string;
}) {
  const [v, setV] = useState(initial ?? "");
  const [acct, setAcct] = useState(defaultAccount ?? accounts?.[0]?.email ?? "");
  const go = () => v.trim() && onSubmit(v.trim(), accounts ? acct : undefined);
  return (
    <div className="sheet-backdrop" onClick={onCancel}>
      <div className="sheet promptsheet" onClick={(e) => e.stopPropagation()}>
        <div className="form">
          <b style={{ fontSize: 15 }}>{title}</b>
          {accounts && (
            <label>Create in account
              <select value={acct} onChange={(e) => setAcct(e.target.value)}>
                {accounts.map((a) => <option key={a.email} value={a.email}>{a.display_name} ({a.email})</option>)}
              </select>
            </label>
          )}
          <input autoFocus value={v} onChange={(e) => setV(e.target.value)} placeholder="Folder name"
            onKeyDown={(e) => { if (e.key === "Enter") go(); if (e.key === "Escape") onCancel(); }} />
          <div className="prompt-actions">
            <button onClick={onCancel}>Cancel</button>
            <button className="primary" disabled={!v.trim()} onClick={go}>{confirm}</button>
          </div>
        </div>
      </div>
    </div>
  );
}
