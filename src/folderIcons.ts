/** Matte folder tiles: which glyph and colour a folder gets. Decided by the folder's *role* (its well-known name from
 *  Microsoft Graph, a Gmail system label, or an IMAP SPECIAL-USE flag), never by its display name, so renamed or
 *  translated folders keep the right tile. Anything without a role (custom folders, Gmail labels) is a plain folder. */

export type FolderRole =
  | "inbox" | "allinboxes" | "drafts" | "sent" | "archive" | "deleted" | "junk" | "outbox" | "rss" | "folder" | "syncissues"
  | "calendar" | "contacts";

/** Glyph paths (24x24, stroke only) and tile colours; the same in every theme (white glyphs pass contrast on all). */
export const TILES: Record<FolderRole, { colour: string; path: string }> = {
  inbox: { colour: "#2F6FD6", path: "M3 13l3-8h12l3 8v6H3z M3 13h5l1.5 3h5L16 13h5" },
  allinboxes: { colour: "#5458D6", path: "M3 14l2.5-6h13L21 14v5H3z M3 14h5l1.5 2.5h5L16 14h5 M6.5 4.5h11" },
  drafts: { colour: "#A9741A", path: "M14.5 4.5l5 5L9 20H4v-5z M12.5 6.5l5 5" },
  sent: { colour: "#1E8C74", path: "M21 3L3 10.5l7 3l3 7z M21 3L10 13.5" },
  archive: { colour: "#7550D0", path: "M3 4h18v5H3z M4 9v10a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1V9 M10 13h4" },
  deleted: { colour: "#C9473B", path: "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v5 M14 11v5" },
  junk: { colour: "#B8631C", path: "M12 3l10 18H2z M12 10v4 M12 17.5h0.01" },
  outbox: { colour: "#1F7FAF", path: "M3 13h5l1.5 3h5L16 13h5v6H3z M12 3v9 M8.5 6.5L12 3l3.5 3.5" },
  rss: { colour: "#B8457A", path: "M5 11a8 8 0 0 1 8 8 M5 5a14 14 0 0 1 14 14 M6 18h0.01" },
  folder: { colour: "#94782A", path: "M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" },
  syncissues: { colour: "#5E6878", path: "M20 12a8 8 0 0 1-14 5.3 M4 12a8 8 0 0 1 14-5.3 M18 3v4h-4 M6 21v-4h4" },
  calendar: { colour: "#C9473B", path: "M4 6h16v14H4z M4 10h16 M8 3v5 M16 3v5 M8 14h2 M14 14h2 M8 17h2" },
  contacts: { colour: "#1E8C74", path: "M12 12a4 4 0 1 0 0-8a4 4 0 1 0 0 8z M4.5 20a7.5 7.5 0 0 1 15 0" },
};

/** Role names the backend stores in `well_known_name`: Microsoft Graph well-known folders (also used for the Gmail
 *  system labels INBOX/SENT/DRAFT/TRASH/SPAM and the synthetic Archive, and for IMAP SPECIAL-USE \Sent, \Drafts,
 *  \Trash, \Junk, \All, \Archive, \Flagged, \Important). Microsoft names without a tile of their own are plain folders. */
const BY_WELL_KNOWN: Record<string, FolderRole> = {
  inbox: "inbox",
  drafts: "drafts",
  sentitems: "sent",
  archive: "archive",
  deleteditems: "deleted",
  junkemail: "junk",
  outbox: "outbox",
  rssfeeds: "rss",
  syncissues: "syncissues",
  conflicts: "syncissues",
  localfailures: "syncissues",
  serverfailures: "syncissues",
  conversationhistory: "folder",
  clutter: "folder",
  scheduled: "folder",
  flagged: "folder",
  important: "folder",
};

/** The tile for one well-known name (null when it has none: a custom folder or a Gmail label). */
export function roleOf(wellKnown: string | null | undefined): FolderRole | null {
  if (!wellKnown) return null;
  return BY_WELL_KNOWN[wellKnown.toLowerCase()] ?? null;
}

export type RoleSource = { id: string; remote_id: string; parent_remote_id: string | null; well_known: string | null };

/** The tile of every folder of an account. Sync Issues' subfolders (any depth) share its tile; everything else without
 *  a role is a plain folder, whatever its name says. */
export function folderRoles(folders: RoleSource[]): Map<string, FolderRole> {
  const byRemote = new Map(folders.map((f) => [f.remote_id, f]));
  const out = new Map<string, FolderRole>();
  for (const f of folders) {
    let role = roleOf(f.well_known);
    if (!role) {
      let p = f.parent_remote_id ? byRemote.get(f.parent_remote_id) : undefined;
      for (let hops = 0; p && hops < 32; hops++) {
        if (roleOf(p.well_known) === "syncissues") { role = "syncissues"; break; }
        p = p.parent_remote_id ? byRemote.get(p.parent_remote_id) : undefined;
      }
    }
    out.set(f.id, role ?? "folder");
  }
  return out;
}
