import { describe, expect, it } from "vitest";
import { bulkFlag, bulkRead, clickSelect, prune, selectAll, single } from "./selection";

const order = ["a", "b", "c", "d", "e"];
const ids = (s: { ids: Set<string> }) => [...s.ids].sort();

describe("message selection", () => {
  it("plain click selects only that message", () => {
    expect(ids(clickSelect(order, selectAll(order), "c", {}))).toEqual(["c"]);
  });
  it("ctrl+click adds and removes", () => {
    let s = clickSelect(order, single("a"), "c", { ctrl: true });
    expect(ids(s)).toEqual(["a", "c"]);
    s = clickSelect(order, s, "a", { ctrl: true });
    expect(ids(s)).toEqual(["c"]);
  });
  it("shift+click selects a range in either direction from the anchor", () => {
    expect(ids(clickSelect(order, single("b"), "d", { shift: true }))).toEqual(["b", "c", "d"]);
    expect(ids(clickSelect(order, single("d"), "a", { shift: true }))).toEqual(["a", "b", "c", "d"]);
  });
  it("shift keeps the anchor, so the range can be changed", () => {
    let s = clickSelect(order, single("c"), "e", { shift: true });
    s = clickSelect(order, s, "a", { shift: true });
    expect(ids(s)).toEqual(["a", "b", "c"]);
  });
  it("ctrl+shift adds a range to what is selected", () => {
    let s = clickSelect(order, single("a"), "d", { ctrl: true });
    s = clickSelect(order, s, "e", { ctrl: true, shift: true });
    expect(ids(s)).toEqual(["a", "d", "e"]);
  });
  it("shift without a usable anchor acts like a plain click", () => {
    expect(ids(clickSelect(order, { ids: new Set(), anchor: "gone" }, "b", { shift: true }))).toEqual(["b"]);
  });
  it("select all, then prune what left the list", () => {
    const s = prune(["a", "c"], selectAll(order));
    expect(ids(s)).toEqual(["a", "c"]);
    expect(prune(["x"], single("a")).anchor).toBeNull();
  });
  it("bulk read/flag pick the useful direction", () => {
    expect(bulkRead([{ is_read: true }, { is_read: false }])).toBe(true);
    expect(bulkRead([{ is_read: true }])).toBe(false);
    expect(bulkFlag([{ is_flagged: true }, { is_flagged: true }])).toBe(false);
  });
});
