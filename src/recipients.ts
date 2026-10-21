/** Recipient fields hold addresses separated by commas or semicolons; suggestions complete the last one. */
export const lastToken = (value: string) => value.slice(Math.max(value.lastIndexOf(","), value.lastIndexOf(";")) + 1).trim();

/** Replaces the address being typed with `email`, ready for the next one. */
export function replaceLastToken(value: string, email: string) {
  const cut = Math.max(value.lastIndexOf(","), value.lastIndexOf(";")) + 1;
  const head = value.slice(0, cut).trimEnd();
  return `${head ? `${head} ` : ""}${email}, `;
}

export const initials = (n?: string | null) => (n || "?").split(/\s+/).filter(Boolean).map((w) => w[0]).slice(0, 2).join("").toUpperCase() || "?";
