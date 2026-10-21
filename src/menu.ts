/** Where a context menu goes so that it stays fully on screen (flips left/up near the edges). */
export function placeMenu(x: number, y: number, w: number, h: number, vw: number, vh: number, margin = 6) {
  const left = x + w + margin > vw ? Math.max(margin, x - w) : x;
  const top = y + h + margin > vh ? Math.max(margin, vh - h - margin) : y;
  return { left, top };
}

/** Where a submenu goes: beside its item, on the right unless that would leave the window, and never below the bottom. */
export function placeSubmenu(item: { left: number; right: number; top: number }, w: number, h: number, vw: number, vh: number, margin = 6) {
  const left = item.right + w + margin <= vw ? item.right - 2 : Math.max(margin, item.left - w + 2);
  const top = Math.max(margin, Math.min(item.top - 4, vh - h - margin));
  return { left, top };
}

/** Next enabled item for arrow-key navigation (wraps around). */
export function stepItem(disabled: boolean[], from: number, dir: 1 | -1) {
  const n = disabled.length;
  for (let i = 1; i <= n; i++) {
    const j = (((from + dir * i) % n) + n) % n;
    if (!disabled[j]) return j;
  }
  return from;
}

/** Where keyboard focus starts in a menu: the first enabled item that isn't destructive, so the first Enter can never
 *  delete, block or report anything. -1 when every enabled item is destructive: the menu itself takes focus and the
 *  arrow keys pick an item on purpose. */
export function initialFocus(items: { disabled: boolean; destructive: boolean }[]) {
  return items.findIndex((it) => !it.disabled && !it.destructive);
}
