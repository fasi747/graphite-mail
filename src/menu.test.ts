import { describe, expect, it } from "vitest";
import { placeMenu, placeSubmenu, stepItem } from "./menu";

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

describe("submenu placement", () => {
  it("opens to the right, or to the left near the edge, and stays above the bottom", () => {
    expect(placeSubmenu({ left: 100, right: 300, top: 200 }, 180, 120, 1200, 800)).toEqual({ left: 298, top: 196 });
    expect(placeSubmenu({ left: 900, right: 1100, top: 200 }, 180, 120, 1200, 800).left).toBe(722); // flipped left
    expect(placeSubmenu({ left: 900, right: 1100, top: 760 }, 180, 120, 1200, 800).top).toBe(674); // pushed up
    expect(placeSubmenu({ left: 10, right: 150, top: 5 }, 400, 900, 320, 480)).toEqual({ left: 6, top: 6 }); // tiny window
  });
});
