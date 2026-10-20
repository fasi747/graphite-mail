import { describe, expect, it } from "vitest";
import { placeMenu, stepItem } from "./menu";

describe("context menu", () => {
  it("opens at the pointer when it fits", () => {
    expect(placeMenu(100, 100, 200, 300, 1000, 800)).toEqual({ left: 100, top: 100 });
  });
  it("flips left at the right edge and moves up at the bottom", () => {
    expect(placeMenu(900, 700, 200, 300, 1000, 800)).toEqual({ left: 700, top: 494 });
  });
  it("never goes off the top-left in a tiny window", () => {
    expect(placeMenu(50, 50, 400, 900, 300, 600)).toEqual({ left: 6, top: 6 });
  });
  it("arrow keys skip disabled items and wrap", () => {
    const off = [false, true, false, true];
    expect(stepItem(off, 0, 1)).toBe(2);
    expect(stepItem(off, 2, 1)).toBe(0);
    expect(stepItem(off, 0, -1)).toBe(2);
  });
});
