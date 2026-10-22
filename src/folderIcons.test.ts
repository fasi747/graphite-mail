import { describe, expect, it } from "vitest";
import { TILES, folderRoles, roleOf } from "./folderIcons";

const f = (id: string, well_known: string | null, parent: string | null = null) => ({ id, remote_id: `r-${id}`, parent_remote_id: parent ? `r-${parent}` : null, well_known });

describe("folder tiles by role", () => {
  it("maps Microsoft well-known folders, whatever they are called", () => {
    expect(roleOf("inbox")).toBe("inbox");
    expect(roleOf("drafts")).toBe("drafts");
    expect(roleOf("sentitems")).toBe("sent");
    expect(roleOf("archive")).toBe("archive");
    expect(roleOf("deleteditems")).toBe("deleted");
    expect(roleOf("junkemail")).toBe("junk");
    expect(roleOf("outbox")).toBe("outbox");
    expect(roleOf("syncissues")).toBe("syncissues");
    expect(roleOf("conflicts")).toBe("syncissues");
    expect(roleOf("conversationhistory")).toBe("folder");
    expect(roleOf("JunkEmail")).toBe("junk"); // case doesn't matter
  });

  it("maps Gmail system labels and IMAP special-use flags through the names the backend stores", () => {
    // Gmail: INBOX/SENT/DRAFT/TRASH/SPAM become inbox/sentitems/drafts/deleteditems/junkemail; All Mail is "archive".
    for (const [stored, role] of [["inbox", "inbox"], ["sentitems", "sent"], ["drafts", "drafts"], ["deleteditems", "deleted"], ["junkemail", "junk"], ["archive", "archive"]] as const) {
      expect(roleOf(stored)).toBe(role);
    }
    // IMAP \Flagged and \Important have no tile of their own.
    expect(roleOf("flagged")).toBe("folder");
    expect(roleOf("important")).toBe("folder");
  });

  it("gives custom folders, Gmail labels and localised names the plain folder tile", () => {
    expect(roleOf(null)).toBeNull();
    expect(roleOf("")).toBeNull();
    const roles = folderRoles([
      { ...f("de", "inbox"), remote_id: "AAMk-1" }, // "Posteingang" on a German mailbox: still the inbox by role
      f("projects", null), f("label", null), f("trash-named", null), // a user folder called "Trash" is still a plain folder
    ]);
    expect(roles.get("de")).toBe("inbox");
    expect([roles.get("projects"), roles.get("label"), roles.get("trash-named")]).toEqual(["folder", "folder", "folder"]);
  });

  it("lets Sync Issues' subfolders share its tile, but not other subfolders", () => {
    const roles = folderRoles([
      f("sync", "syncissues"), f("conflicts", "conflicts", "sync"), f("local", null, "sync"), f("deep", null, "local"),
      f("inbox", "inbox"), f("inbox-child", null, "inbox"),
      f("orphan", null, "missing-parent"),
    ]);
    expect(roles.get("conflicts")).toBe("syncissues");
    expect(roles.get("local")).toBe("syncissues");
    expect(roles.get("deep")).toBe("syncissues");
    expect(roles.get("inbox-child")).toBe("folder");
    expect(roles.get("orphan")).toBe("folder");
  });

  it("has a glyph and the approved colour for every role", () => {
    expect(TILES.inbox.colour).toBe("#2F6FD6");
    expect(TILES.allinboxes.colour).toBe("#5458D6");
    expect(TILES.folder.colour).toBe("#94782A");
    expect(TILES.calendar.colour).toBe("#C9473B");
    expect(TILES.contacts.colour).toBe("#1E8C74");
    for (const t of Object.values(TILES)) {
      expect(t.colour).toMatch(/^#[0-9A-F]{6}$/);
      expect(t.path).toMatch(/^M/);
    }
  });
});
