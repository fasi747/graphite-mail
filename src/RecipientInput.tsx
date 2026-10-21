import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { lastToken, replaceLastToken } from "./recipients";

type Suggestion = { name: string; email: string };

/** A To/Cc field that suggests people from the sending account's contacts (Microsoft 365 accounts). */
export default function RecipientInput({ id, value, onChange, account, label }: {
  id?: string; value: string; onChange: (v: string) => void; account: string; label: string;
}) {
  const [items, setItems] = useState<Suggestion[]>([]);
  const [active, setActive] = useState(0);
  // Suggestions follow typing only: a field that opens with addresses in it (a reply, "New email") stays quiet.
  const [typing, setTyping] = useState(false);
  const asked = useRef(0);
  const listId = `${id ?? label}-suggestions`;

  useEffect(() => {
    const q = lastToken(value);
    if (!typing || q.length < 2) { setItems([]); return; }
    const mine = ++asked.current;
    const t = setTimeout(() => {
      invoke<Suggestion[]>("suggest_recipients", { query: q, account })
        .then((s) => { if (mine === asked.current) { setItems(s.filter((x) => !value.toLowerCase().includes(`${x.email.toLowerCase()},`))); setActive(0); } })
        .catch(() => setItems([]));
    }, 120);
    return () => clearTimeout(t);
  }, [value, account, typing]);

  const pick = (s: Suggestion) => { asked.current++; setItems([]); onChange(replaceLastToken(value, s.email)); };
  const open = items.length > 0;

  return (
    <span className="recip">
      <input id={id} value={value} aria-label={label} role="combobox" aria-expanded={open} aria-controls={listId} aria-autocomplete="list"
        aria-activedescendant={open ? `${listId}-${active}` : undefined} autoComplete="off" spellCheck={false}
        onChange={(e) => { setTyping(true); onChange(e.target.value); }}
        onBlur={() => { asked.current++; setTyping(false); setItems([]); }}
        onKeyDown={(e) => {
          if (!open) return;
          if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => (a + 1) % items.length); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => (a - 1 + items.length) % items.length); }
          else if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); e.stopPropagation(); pick(items[active]); }
          else if (e.key === "Escape") { e.stopPropagation(); setItems([]); }
        }} />
      {open && (
        <ul className="suggest" role="listbox" id={listId} aria-label={`${label} suggestions`}>
          {items.map((s, i) => (
            <li key={s.email} id={`${listId}-${i}`} role="option" aria-selected={i === active} className={i === active ? "on" : ""}
              onMouseDown={(e) => { e.preventDefault(); pick(s); }} onMouseEnter={() => setActive(i)}>
              <b>{s.name}</b><span>{s.email}</span>
            </li>
          ))}
        </ul>
      )}
    </span>
  );
}
