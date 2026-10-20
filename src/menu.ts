/** Where a context menu goes so that it stays fully on screen (flips left/up near the edges). */
export function placeMenu(x: number, y: number, w: number, h: number, vw: number, vh: number, margin = 6) {
  const left = x + w + margin > vw ? Math.max(margin, x - w) : x;
  const top = y + h + margin > vh ? Math.max(margin, vh - h - margin) : y;
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
