/** Multi-select in the message list, kept free of React so it can be tested on its own. */
export type Selection = { ids: Set<string>; anchor: string | null };

export const single = (id: string): Selection => ({ ids: new Set([id]), anchor: id });

/**
 * A click on message `id`: plain = only it; Ctrl/Cmd (or its checkbox) = add or remove it;
 * Shift = everything from the anchor to it (added to the current ones when Ctrl is held too).
 */
export function clickSelect(order: string[], cur: Selection, id: string, mods: { ctrl?: boolean; shift?: boolean }): Selection {
  if (mods.shift && cur.anchor && order.includes(cur.anchor) && order.includes(id)) {
    const [a, b] = [order.indexOf(cur.anchor), order.indexOf(id)].sort((x, y) => x - y);
    const ids = new Set(mods.ctrl ? cur.ids : []);
    for (const x of order.slice(a, b + 1)) ids.add(x);
    return { ids, anchor: cur.anchor };
  }
  if (mods.ctrl) {
    const ids = new Set(cur.ids);
    if (ids.has(id)) ids.delete(id); else ids.add(id);
    return { ids, anchor: id };
  }
  return single(id);
}

export const selectAll = (order: string[]): Selection => ({ ids: new Set(order), anchor: order[0] ?? null });

/** Drops messages that are no longer listed (moved, deleted, another folder). */
export function prune(order: string[], cur: Selection): Selection {
  const listed = new Set(order);
  const ids = new Set([...cur.ids].filter((x) => listed.has(x)));
  return ids.size === cur.ids.size ? cur : { ids, anchor: cur.anchor && listed.has(cur.anchor) ? cur.anchor : null };
}

/** For a group of messages: "mark read" unless every one is read already; "flag" unless every one is flagged. */
export const bulkRead = (ms: { is_read: boolean }[]) => ms.some((m) => !m.is_read);
export const bulkFlag = (ms: { is_flagged: boolean }[]) => ms.some((m) => !m.is_flagged);
