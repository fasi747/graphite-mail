/** What the Contacts view's right-click menu offers, kept free of React so each rule can be tested on its own. */
import type { MenuItem } from "./ContextMenu";

export type MenuContact = {
  id: string; account_email: string; name: string; emails: string[];
  phones: { kind: string; number: string }[]; favourite: boolean;
};

export type ContactActions = {
  newEmail: (to: string[], from: string) => void;
  newMeeting: (attendees: string[], account: string) => void;
  searchMail: (addresses: string[]) => void;
  copy: (text: string, note: string) => void;
  favourite: (ids: string[], on: boolean) => void;
  exportVcf: (contacts: MenuContact[]) => void;
  block: (account: string, address: string) => void;
  createRule: (account: string, address: string) => void;
};

/** "Name <email>", quoting the name when it holds characters that mean something in an address (RFC 5322). */
export function nameAndAddress(name: string, email: string) {
  const n = name.trim();
  if (!n || n.toLowerCase() === email.toLowerCase()) return email;
  return /[()<>[\]:;@\\,."]/.test(n) ? `"${n.replace(/(["\\])/g, "\\$1")}" <${email}>` : `${n} <${email}>`;
}

/** Each contact's first address, once each. */
export function firstAddresses(cs: MenuContact[]) {
  const seen = new Map<string, string>(); // lower-case → as first written
  for (const c of cs) if (c.emails.length && !seen.has(c.emails[0].toLowerCase())) seen.set(c.emails[0].toLowerCase(), c.emails[0]);
  return [...seen.values()];
}

/** Joins sections, putting a separator line before each section after the first that has items. */
function sections(...groups: (MenuItem | null | false)[][]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const g of groups) {
    const items = g.filter(Boolean) as MenuItem[];
    if (!items.length) continue;
    out.push(...items.map((it, i) => (i === 0 && out.length ? { ...it, separator: true } : it)));
  }
  return out;
}

/** An item acting on one address: direct with one, a submenu to choose with several. */
function perAddress(label: string, icon: string, emails: string[], act: (e: string) => void, show: (e: string) => string = (e) => e, destructive = false): MenuItem {
  return emails.length > 1 ? { label, icon, destructive, submenu: emails.map((e) => ({ label: show(e), onClick: () => act(e) })) } : { label, icon, destructive, onClick: () => act(emails[0]) };
}

export function contactMenu(cs: MenuContact[], a: ContactActions): MenuItem[] {
  if (cs.length === 0) return [];
  if (cs.length > 1) {
    const to = firstAddresses(cs);
    const from = cs.find((c) => c.emails.length)?.account_email ?? cs[0].account_email;
    return sections(
      to.length ? [
        { label: `New email to all (${to.length})`, icon: "compose", onClick: () => a.newEmail(to, from) },
        { label: `New meeting with all (${to.length})`, icon: "meeting", onClick: () => a.newMeeting(to, from) },
        { label: "Copy all addresses", icon: "copy", onClick: () => a.copy(to.join("; "), `${to.length} addresses copied`) },
      ] : [],
      [{ label: `Export selected as vCard (${cs.length})…`, icon: "export", onClick: () => a.exportVcf(cs) }],
    );
  }
  const c = cs[0];
  const e = c.emails;
  const has = e.length > 0;
  return sections(
    has ? [
      perAddress("New email", "compose", e, (x) => a.newEmail([x], c.account_email)),
      perAddress("New meeting", "meeting", e, (x) => a.newMeeting([x], c.account_email)),
      { label: "Search mail with this person", icon: "search", onClick: () => a.searchMail(e) },
    ] : [],
    [
      has && perAddress("Copy email address", "copy", e, (x) => a.copy(x, "Email address copied")),
      has && perAddress("Copy name and address", "copy", e, (x) => a.copy(nameAndAddress(c.name, x), "Name and address copied"), (x) => nameAndAddress(c.name, x)),
      c.phones.length > 0 && (c.phones.length > 1
        ? { label: "Copy phone number", icon: "phone", submenu: c.phones.map((p) => ({ label: `${p.number} (${p.kind})`, onClick: () => a.copy(p.number, "Phone number copied") })) }
        : { label: "Copy phone number", icon: "phone", onClick: () => a.copy(c.phones[0].number, "Phone number copied") }),
    ],
    [
      { label: c.favourite ? "Remove from favourites" : "Add to favourites", icon: "star", onClick: () => a.favourite([c.id], !c.favourite) },
      { label: "Export as vCard…", icon: "export", onClick: () => a.exportVcf([c]) },
    ],
    has ? [
      perAddress("Block sender", "block", e, (x) => a.block(c.account_email, x), undefined, true),
      perAddress("Create rule for this sender…", "rules", e, (x) => a.createRule(c.account_email, x)),
    ] : [],
  );
}

/** A file name for the exported vCard. */
export function vcfName(cs: { name: string }[]) {
  if (cs.length !== 1) return "contacts.vcf";
  const base = cs[0].name.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 80);
  return `${base || "contact"}.vcf`;
}
