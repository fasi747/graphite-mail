import { describe, expect, it, vi } from "vitest";
import { ContactActions, contactMenu, MenuContact, nameAndAddress, vcfName } from "./contactMenu";
import type { MenuItem } from "./ContextMenu";

const c = (over: Partial<MenuContact> = {}): MenuContact =>
  ({ id: "c1", account_email: "me@corp.com", name: "Ana Diaz", emails: ["ana@corp.com"], phones: [{ kind: "work", number: "+1 555 0100" }], favourite: false, ...over });
const acts = (): ContactActions => ({ newEmail: vi.fn(), newMeeting: vi.fn(), searchMail: vi.fn(), copy: vi.fn(), favourite: vi.fn(), exportVcf: vi.fn(), block: vi.fn(), createRule: vi.fn() });
const labels = (m: MenuItem[]) => m.map((i) => i.label);
const item = (m: MenuItem[], l: string) => m.find((i) => i.label === l)!;

describe("contact menu", () => {
  it("offers everything for a contact with one address and one phone", () => {
    const a = acts();
    const m = contactMenu([c()], a);
    expect(labels(m)).toEqual(["New email", "New meeting", "Search mail with this person", "Copy email address", "Copy name and address",
      "Copy phone number", "Add to favourites", "Export as vCard…", "Block sender", "Create rule for this sender…"]);
    expect(m.filter((i) => i.separator).map((i) => i.label)).toEqual(["Copy email address", "Add to favourites", "Block sender"]);
    expect(m.some((i) => i.submenu)).toBe(false);
    item(m, "New email").onClick!();
    expect(a.newEmail).toHaveBeenCalledWith(["ana@corp.com"], "me@corp.com");
    item(m, "New meeting").onClick!();
    expect(a.newMeeting).toHaveBeenCalledWith(["ana@corp.com"], "me@corp.com");
    item(m, "Copy name and address").onClick!();
    expect(a.copy).toHaveBeenCalledWith("Ana Diaz <ana@corp.com>", expect.any(String));
    item(m, "Copy phone number").onClick!();
    expect(a.copy).toHaveBeenCalledWith("+1 555 0100", expect.any(String));
    item(m, "Block sender").onClick!();
    expect(a.block).toHaveBeenCalledWith("me@corp.com", "ana@corp.com");
    item(m, "Create rule for this sender…").onClick!();
    expect(a.createRule).toHaveBeenCalledWith("me@corp.com", "ana@corp.com");
    item(m, "Add to favourites").onClick!();
    expect(a.favourite).toHaveBeenCalledWith(["c1"], true);
  });

  it("lets the user pick the address when there are several", () => {
    const a = acts();
    const m = contactMenu([c({ emails: ["ana@corp.com", "ana@home.com"] })], a);
    for (const l of ["New email", "New meeting", "Copy email address", "Copy name and address", "Block sender", "Create rule for this sender…"]) {
      expect(item(m, l).submenu, l).toBeDefined();
    }
    expect(labels(item(m, "New email").submenu!)).toEqual(["ana@corp.com", "ana@home.com"]);
    expect(labels(item(m, "Copy name and address").submenu!)).toEqual(["Ana Diaz <ana@corp.com>", "Ana Diaz <ana@home.com>"]);
    item(m, "New email").submenu![1].onClick!();
    expect(a.newEmail).toHaveBeenCalledWith(["ana@home.com"], "me@corp.com");
    item(m, "Search mail with this person").onClick!();
    expect(a.searchMail).toHaveBeenCalledWith(["ana@corp.com", "ana@home.com"]);
  });

  it("hides the email items for a contact without an address, and the phone item without a phone", () => {
    expect(labels(contactMenu([c({ emails: [] })], acts()))).toEqual(["Copy phone number", "Add to favourites", "Export as vCard…"]);
    expect(labels(contactMenu([c({ emails: [], phones: [] })], acts()))).toEqual(["Add to favourites", "Export as vCard…"]);
    const m = contactMenu([c({ emails: [], phones: [] })], acts());
    expect(m[0].separator).toBeFalsy(); // no separator line at the top when the first sections are empty
    expect(labels(contactMenu([c({ phones: [] })], acts()))).not.toContain("Copy phone number");
  });

  it("puts several phone numbers in a submenu, and says remove for a favourite", () => {
    const m = contactMenu([c({ phones: [{ kind: "work", number: "1" }, { kind: "mobile", number: "2" }], favourite: true })], acts());
    expect(labels(item(m, "Copy phone number").submenu!)).toEqual(["1 (work)", "2 (mobile)"]);
    expect(labels(m)).toContain("Remove from favourites");
  });

  it("offers group actions for a selection, and only those that apply", () => {
    const a = acts();
    const two = [c(), c({ id: "c2", name: "Ben", emails: ["ben@corp.com", "b@home.com"] }), c({ id: "c3", name: "Ana again", emails: ["ANA@corp.com"] }), c({ id: "c4", emails: [] })];
    const m = contactMenu(two, a);
    expect(labels(m)).toEqual(["New email to all (2)", "New meeting with all (2)", "Copy all addresses", "Export selected as vCard (4)…"]);
    item(m, "Copy all addresses").onClick!();
    expect(a.copy).toHaveBeenCalledWith("ana@corp.com; ben@corp.com", "2 addresses copied");
    item(m, "New email to all (2)").onClick!();
    expect(a.newEmail).toHaveBeenCalledWith(["ana@corp.com", "ben@corp.com"], "me@corp.com");
    item(m, "Export selected as vCard (4)…").onClick!();
    expect((a.exportVcf as any).mock.calls[0][0]).toHaveLength(4);
    expect(labels(contactMenu([c({ emails: [] }), c({ id: "c2", emails: [] })], acts()))).toEqual(["Export selected as vCard (2)…"]);
  });

  it("writes names and file names safely", () => {
    expect(nameAndAddress("Ana Diaz", "ana@corp.com")).toBe("Ana Diaz <ana@corp.com>");
    expect(nameAndAddress("O'Brien, Seán", "s@x.ie")).toBe('"O\'Brien, Seán" <s@x.ie>');
    expect(nameAndAddress('Say "hi"', "h@x.com")).toBe('"Say \\"hi\\"" <h@x.com>');
    expect(nameAndAddress("ana@corp.com", "ana@corp.com")).toBe("ana@corp.com");
    expect(vcfName([{ name: "Ana / Diaz: CFO?" }])).toBe("Ana Diaz CFO.vcf");
    expect(vcfName([{ name: "a" }, { name: "b" }])).toBe("contacts.vcf");
    expect(vcfName([{ name: "***" }])).toBe("contact.vcf");
  });
});
