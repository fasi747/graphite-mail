/** Colour themes. "system" follows the desktop's light/dark setting. */
export const THEMES = [
  { id: "white", label: "White", dark: false },
  { id: "lightgrey", label: "Light Grey", dark: false },
  { id: "mediumgrey", label: "Medium Grey", dark: false },
  { id: "darkgrey", label: "Dark Grey", dark: true },
  { id: "black", label: "Black", dark: true },
  { id: "colourful", label: "Colourful", dark: false },
] as const;
export type ThemeId = (typeof THEMES)[number]["id"];
export type ThemeChoice = ThemeId | "system";

export const isChoice = (t: unknown): t is ThemeChoice => t === "system" || THEMES.some((x) => x.id === t);

/** The theme actually shown for a choice. */
export const resolveTheme = (choice: ThemeChoice, systemDark: boolean): ThemeId =>
  choice === "system" ? (systemDark ? "darkgrey" : "white") : choice;

export const isDark = (t: ThemeId) => THEMES.find((x) => x.id === t)!.dark;

const systemDark = () => { try { return matchMedia("(prefers-color-scheme: dark)").matches; } catch { return false; } };

/** Puts a theme on the page (and remembers it locally, so the next start shows it before settings load). */
export function applyTheme(choice: ThemeChoice) {
  const t = resolveTheme(choice, systemDark());
  document.documentElement.dataset.theme = t;
  document.documentElement.dataset.mode = isDark(t) ? "dark" : "light";
  try { localStorage.setItem("themeChoice", choice); } catch { /* ignore */ }
  return t;
}

export function cachedChoice(): ThemeChoice {
  try { const c = localStorage.getItem("themeChoice"); if (isChoice(c)) return c; } catch { /* ignore */ }
  return "system";
}
