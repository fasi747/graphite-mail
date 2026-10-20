import { describe, expect, it } from "vitest";
import { THEMES, isChoice, isDark, resolveTheme } from "./theme";

describe("themes", () => {
  it("offers the six themes", () => {
    expect(THEMES.map((t) => t.label)).toEqual(["White", "Light Grey", "Medium Grey", "Dark Grey", "Black", "Colourful"]);
  });
  it("system follows the desktop", () => {
    expect(resolveTheme("system", true)).toBe("darkgrey");
    expect(resolveTheme("system", false)).toBe("white");
    expect(resolveTheme("colourful", true)).toBe("colourful");
  });
  it("knows which themes are dark", () => {
    expect(isDark("black") && isDark("darkgrey")).toBe(true);
    expect(isDark("colourful") || isDark("white")).toBe(false);
  });
  it("rejects old or unknown values", () => {
    expect(isChoice("dark")).toBe(false);
    expect(isChoice("system") && isChoice("lightgrey")).toBe(true);
  });
});
