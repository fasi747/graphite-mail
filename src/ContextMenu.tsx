import React, { useLayoutEffect, useRef, useState } from "react";
import Icon from "./Icon";
import { initialFocus, placeMenu, placeSubmenu, stepItem } from "./menu";

export type MenuItem = {
  label: string; icon?: string; onClick?: () => void; disabled?: boolean; separator?: boolean;
  /** Choices shown beside the item (e.g. which address to use). */
  submenu?: MenuItem[];
  /** Deletes, blocks or reports something: never the item the keyboard starts on, so a stray Enter can't do it. */
  destructive?: boolean;
};

const offs = (items: MenuItem[]) => items.map((i) => !!i.disabled);
/** Where the keyboard starts: the first enabled, non-destructive item; -1 (the menu itself) when there is none. */
export const firstOn = (items: MenuItem[]) => initialFocus(items.map((i) => ({ disabled: !!i.disabled, destructive: !!i.destructive })));

/** One panel of a menu. Placement is decided once its size is known, so it can stay inside the window. */
function Panel({ items, label, place, focus, takeFocus, open, onKey, onHover, onPick, panel }: {
  items: MenuItem[]; label: string; place: (w: number, h: number) => { left: number; top: number }; focus: number; takeFocus: boolean;
  open?: number; onKey: (e: React.KeyboardEvent) => void; onHover: (i: number, el: HTMLElement) => void; onPick: (i: number, el: HTMLElement) => void;
  panel?: React.RefObject<HTMLDivElement | null>;
}) {
  const own = useRef<HTMLDivElement>(null);
  const ref = panel ?? own;
  const [pos, setPos] = useState({ left: 0, top: 0, ready: false });
  useLayoutEffect(() => {
    const el = ref.current;
    if (el) setPos({ ...place(el.offsetWidth, el.offsetHeight), ready: true });
  }, []);
  useLayoutEffect(() => {
    if (takeFocus) ((focus >= 0 && ref.current?.querySelectorAll<HTMLElement>("[role=menuitem]")[focus]) || ref.current)?.focus();
  }, [focus, pos.ready, takeFocus]);
  return (
    <div ref={ref} className="menu ctxmenu" role="menu" tabIndex={-1} aria-label={label} onKeyDown={onKey}
      style={{ left: pos.left, top: pos.top, visibility: pos.ready ? "visible" : "hidden" }} onMouseDown={(e) => e.stopPropagation()}>
      {items.map((it, i) => (
        <div key={i} role="menuitem" tabIndex={-1} aria-disabled={it.disabled || undefined}
          aria-haspopup={it.submenu ? "menu" : undefined} aria-expanded={it.submenu ? open === i : undefined}
          className={`${it.separator ? "sep" : ""} ${it.disabled ? "off" : ""} ${i === focus ? "focus" : ""}`}
          onMouseEnter={(e) => !it.disabled && onHover(i, e.currentTarget)} onClick={(e) => onPick(i, e.currentTarget)}>
          <span className="ctx-ico">{it.icon && <Icon name={it.icon} size={14} />}</span>
          <span className="ctx-label">{it.label}</span>
          {it.submenu && <span className="ctx-more" aria-hidden>›</span>}
        </div>
      ))}
    </div>
  );
}

/** Right-click menu: stays on screen, Esc closes, arrow keys / Home / End move, Enter or Space chooses.
 *  An item with a submenu opens it with → (or Enter, or hovering); ← or Esc goes back.
 *  The keyboard never starts on a destructive item (Delete, Block, Spam): a first Enter can't do harm. */
export function ContextMenu({ x, y, items, onClose, label = "Message actions" }: { x: number; y: number; items: MenuItem[]; onClose: () => void; label?: string }) {
  const root = useRef<HTMLDivElement>(null);
  const [focus, setFocus] = useState(() => firstOn(items));
  const [sub, setSub] = useState<null | { i: number; rect: DOMRect; keyboard: boolean }>(null);
  const [subFocus, setSubFocus] = useState(0);
  const subItems = sub ? items[sub.i].submenu ?? [] : [];

  const run = (it: MenuItem) => { if (!it.disabled) { onClose(); it.onClick?.(); } };
  const openSub = (i: number, el: HTMLElement, keyboard: boolean) => {
    const s = items[i].submenu;
    if (!s?.length || items[i].disabled) return;
    setSub({ i, rect: el.getBoundingClientRect(), keyboard });
    setSubFocus(firstOn(s));
  };
  const itemEl = (i: number) => (i >= 0 ? root.current?.querySelectorAll<HTMLElement>("[role=menuitem]")[i] : undefined);
  const pick = (i: number, el: HTMLElement, keyboard = false) => (items[i].submenu ? openSub(i, el, keyboard) : run(items[i]));
  const moves = (e: React.KeyboardEvent, list: MenuItem[], set: (f: (n: number) => number) => void) => {
    const k = e.key, off = offs(list);
    if (k === "ArrowDown") set((f) => stepItem(off, f, 1));
    else if (k === "ArrowUp") set((f) => stepItem(off, f < 0 ? 0 : f, -1));
    else if (k === "Home") set(() => stepItem(off, -1, 1));
    else if (k === "End") set(() => stepItem(off, 0, -1));
    else return false;
    e.preventDefault();
    return true;
  };
  const onRootKey = (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (moves(e, items, (f) => { setSub(null); setFocus(f); })) return;
    const k = e.key;
    if (k === "Escape" || k === "Tab") { e.preventDefault(); onClose(); }
    else if (k === "ArrowRight" && items[focus]?.submenu) { e.preventDefault(); const el = itemEl(focus); if (el) openSub(focus, el, true); }
    else if (k === "Enter" || k === " ") { e.preventDefault(); if (focus < 0) return; const el = itemEl(focus); if (el) pick(focus, el, true); }
  };
  const onSubKey = (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (moves(e, subItems, setSubFocus)) return;
    const k = e.key;
    if (k === "Escape" || k === "ArrowLeft") { e.preventDefault(); setSub(null); }
    else if (k === "Tab") { e.preventDefault(); onClose(); }
    else if (k === "Enter" || k === " ") { e.preventDefault(); if (subFocus >= 0 && subItems[subFocus]) run(subItems[subFocus]); }
  };
  return (
    <div className="ctx-backdrop" onMouseDown={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }}>
      <Panel panel={root} items={items} label={label} focus={focus} open={sub?.i} takeFocus={!sub?.keyboard}
        place={(w, h) => placeMenu(x, y, w, h, innerWidth, innerHeight)} onKey={onRootKey}
        onHover={(i, el) => { setFocus(i); if (items[i].submenu) openSub(i, el, false); else setSub(null); }}
        onPick={(i, el) => pick(i, el)} />
      {sub && (
        <Panel key={sub.i} items={subItems} label={items[sub.i].label} focus={subFocus} takeFocus={sub.keyboard}
          place={(w, h) => placeSubmenu(sub.rect, w, h, innerWidth, innerHeight)} onKey={onSubKey}
          onHover={(i) => setSubFocus(i)} onPick={(i) => run(subItems[i])} />
      )}
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
