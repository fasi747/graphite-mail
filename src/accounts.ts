/** Account types and small helpers shared by the screens that show or add accounts. */
export type Acct = { email: string; display_name: string; provider?: string; rules?: boolean; personal?: boolean };

/** Gmail through Google sign-in ("google") or through IMAP/SMTP with an app password ("imap"): labels are folders. */
export const isGmail = (provider?: string) => provider === "google" || provider === "imap";

export const providerLabel = (provider?: string, personal?: boolean) =>
  provider === "google" ? "Gmail" : provider === "imap" ? "Gmail (app password)" : personal ? "Microsoft account (personal)" : "Microsoft 365";

/** Google shows app passwords as four groups of four letters; the spaces are not part of the password. */
export const cleanAppPassword = (s: string) => s.replace(/\s+/g, "");

export const APP_PASSWORD_LENGTH = 16;

export const looksLikeEmail = (s: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(s.trim());

/** Why "Connect" can't be pressed yet (null when it can). */
export function connectHint(email: string, password: string): string | null {
  const code = cleanAppPassword(password);
  if (!email.trim() || !code) return "Enter both fields to continue";
  if (!looksLikeEmail(email)) return "Enter your full Gmail address, for example name@gmail.com";
  if (code.length !== APP_PASSWORD_LENGTH) return `${code.length} of ${APP_PASSWORD_LENGTH} letters`;
  return null;
}

/** A browser sign-in that hasn't come back after this long is stopped, so the screen never stays stuck. */
export const SIGN_IN_TIMEOUT_MS = 2 * 60 * 1000;
