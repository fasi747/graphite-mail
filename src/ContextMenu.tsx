import React, { useLayoutEffect, useRef, useState } from "react";
import Icon from "./Icon";
import { placeMenu, stepItem } from "./menu";

export type MenuItem = { label: string; icon?: string; onClick: () => void; disabled?: boolean; separator?: boolean };

/** Right-click menu: stays on screen, Esc closes, arrow keys / Home / End move, Enter or Space chooses. */
export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y, ready: false });
  const [focus, setFocus] = useState(() => stepItem(items.map((i) => !!i.disabled), -1, 1));
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const p = placeMenu(x, y, el.offsetWidth, el.offsetHeight, innerWidth, innerHeight);
    setPos({ ...p, ready: true });
    el.focus();
  }, [x, y]);
  useLayoutEffect(() => { (ref.current?.querySelectorAll<HTMLElement>("[role=menuitem]")[focus])?.focus(); }, [focus, pos.ready]);
  const disabled = items.map((i) => !!i.disabled);
  const choose = (i: number) => { if (!items[i].disabled) { onClose(); items[i].onClick(); } };
  const onKey = (e: React.KeyboardEvent) => {
    const k = e.key;
    if (k === "Escape" || k === "Tab") { e.preventDefault(); onClose(); }
    else if (k === "ArrowDown") { e.preventDefault(); setFocus((f) => stepItem(disabled, f, 1)); }
    else if (k === "ArrowUp") { e.preventDefault(); setFocus((f) => stepItem(disabled, f, -1)); }
    else if (k === "Home") { e.preventDefault(); setFocus(stepItem(disabled, -1, 1)); }
    else if (k === "End") { e.preventDefault(); setFocus(stepItem(disabled, 0, -1)); }
    else if (k === "Enter" || k === " ") { e.preventDefault(); choose(focus); }
    e.stopPropagation();
  };
  return (
    <div className="ctx-backdrop" onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }}>
      <div ref={ref} className="menu ctxmenu" role="menu" tabIndex={-1} aria-label="Message actions" onKeyDown={onKey}
        style={{ left: pos.left, top: pos.top, visibility: pos.ready ? "visible" : "hidden" }} onMouseDown={(e) => e.stopPropagation()}>
        {items.map((it, i) => (
          <div key={i} role="menuitem" tabIndex={-1} aria-disabled={it.disabled || undefined}
            className={`${it.separator ? "sep" : ""} ${it.disabled ? "off" : ""} ${i === focus ? "focus" : ""}`}
            onMouseEnter={() => !it.disabled && setFocus(i)} onClick={() => choose(i)}>
            <span className="ctx-ico">{it.icon && <Icon name={it.icon} size={14} />}</span>{it.label}
          </div>
        ))}
      </div>
    </div>
  );
}

type PickFolder = { id: string; name: string; depth: number };

/** Choose a folder (Move to… / Copy to…). Type to filter, arrows to pick, Enter to confirm. */
export function FolderPicker({ title, folders, onPick, onCancel }: { title: string; folders: PickFolder[]; onPick: (id: string) => void; onCancel: () => void }) {
  const [q, setQ] = useState("");
  const shown = folders.filter((f) => f.name.toLowerCase().includes(q.trim().toLowerCase()));
  const [i, setI] = useState(0);
  const cur = Math.min(i, shown.length - 1);
  return (
    <div className="sheet-backdrop" onClick={onCancel}>
      <div className="sheet promptsheet" onClick={(e) => e.stopPropagation()}>
        <div className="form">
          <b style={{ fontSize: 15 }}>{title}</b>
          <input autoFocus value={q} placeholder="Find a folder" onChange={(e) => { setQ(e.target.value); setI(0); }}
            onKeyDown={(e) => {
              if (e.key === "Escape") onCancel();
              else if (e.key === "ArrowDown") { e.preventDefault(); setI(Math.min(cur + 1, shown.length - 1)); }
              else if (e.key === "ArrowUp") { e.preventDefault(); setI(Math.max(cur - 1, 0)); }
              else if (e.key === "Enter" && shown[cur]) onPick(shown[cur].id);
            }} />
          <div className="pickerlist" role="listbox">
            {shown.length === 0 && <div className="muted">No folders</div>}
            {shown.map((f, n) => (
              <div key={f.id} role="option" aria-selected={n === cur} className={n === cur ? "on" : ""}
                style={{ paddingLeft: 10 + (q ? 0 : f.depth * 14) }} onMouseEnter={() => setI(n)} onClick={() => onPick(f.id)}>{f.name}</div>
            ))}
          </div>
          <div className="prompt-actions">
            <button onClick={onCancel}>Cancel</button>
            <button className="primary" disabled={!shown[cur]} onClick={() => shown[cur] && onPick(shown[cur].id)}>Choose</button>
          </div>
        </div>
      </div>
    </div>
  );
}

export type QuickRule = { field: "from" | "subject"; text: string; action: "move" | "read" | "delete"; folder: string };

/** "When from / subject contains X → move to folder / mark read / delete". */
export function RuleDialog({ init, folders, busy, error, onSave, onCancel }: {
  init: QuickRule; folders: PickFolder[]; busy: boolean; error: string; onSave: (r: QuickRule) => void; onCancel: () => void;
}) {
  const [r, setR] = useState(init);
  const ok = r.text.trim() && (r.action !== "move" || r.folder);
  return (
    <div className="sheet-backdrop" onClick={onCancel}>
      <div className="sheet promptsheet" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.key === "Escape" && onCancel()}>
        <div className="form">
          <b style={{ fontSize: 15 }}>Create rule</b>
          <label>When
            <select value={r.field} onChange={(e) => setR({ ...r, field: e.target.value as QuickRule["field"] })}>
              <option value="from">the sender contains</option>
              <option value="subject">the subject contains</option>
            </select>
          </label>
          <input autoFocus value={r.text} onChange={(e) => setR({ ...r, text: e.target.value })} aria-label="Text to look for" />
          <label>Then
            <select value={r.action} onChange={(e) => setR({ ...r, action: e.target.value as QuickRule["action"] })}>
              <option value="move">move it to a folder</option>
              <option value="read">mark it as read</option>
              <option value="delete">delete it</option>
            </select>
          </label>
          {r.action === "move" && (
            <label>Folder
              <select value={r.folder} onChange={(e) => setR({ ...r, folder: e.target.value })}>
                <option value="">Choose a folder…</option>
                {folders.map((f) => <option key={f.id} value={f.id}>{"  ".repeat(f.depth)}{f.name}</option>)}
              </select>
            </label>
          )}
          <div className="muted" style={{ fontSize: 12 }}>The rule is saved on the server, so it also works while Graphite Mail is closed. Edit it later under Rules.</div>
          {error && <div className="error">{error}</div>}
          <div className="prompt-actions">
            <button onClick={onCancel}>Cancel</button>
            <button className="primary" disabled={!ok || busy} onClick={() => onSave(r)}>{busy ? "Saving…" : "Create rule"}</button>
          </div>
        </div>
      </div>
    </div>
  );
}
