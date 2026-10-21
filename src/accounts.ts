/** Account types and small helpers shared by the screens that show or add accounts. */
export type Acct = { email: string; display_name: string; provider?: string; rules?: boolean };

/** Gmail through Google sign-in ("google") or through IMAP/SMTP with an app password ("imap"): labels are folders. */
export const isGmail = (provider?: string) => provider === "google" || provider === "imap";

export const providerLabel = (provider?: string) =>
  provider === "google" ? "Gmail" : provider === "imap" ? "Gmail (app password)" : "Microsoft 365";

/** Google shows app passwords as four groups of four letters; the spaces are not part of the password. */
export const cleanAppPassword = (s: string) => s.replace(/\s+/g, "");

export const APP_PASSWORD_LENGTH = 16;

export const looksLikeEmail = (s: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(s.trim());
