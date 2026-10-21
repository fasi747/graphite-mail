// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Mailbox from "./Mailbox";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));
// jsdom has no layout, so the real virtualizer would show no rows: list every message.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () => Array.from({ length: count }, (_, index) => ({ index, key: index, start: index * 76, size: 76 })),
    getTotalSize: () => count * 76,
  }),
}));
const mocked = vi.mocked(invoke);

beforeAll(() => {
  globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as any;
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} })) as any;
});

function backend(accounts: object[], extra: Record<string, (args?: unknown) => unknown> = {}) {
  mocked.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in extra) return extra[cmd](args);
    switch (cmd) {
      case "list_accounts": return accounts;
      case "get_theme": return "white";
      case "contacts_overview": return {
        directory: { total: 1, rows: [{ id: "d1", source: "directory", account_email: "me@corp.com", name: "Ana Diaz", email: "ana@corp.com", subtitle: "CFO" }] },
        personal: { total: 0, rows: [] },
        accounts: [{ email: "me@corp.com", granted: true, directory: "ok", personal: "ok", directory_count: 1, personal_count: 0, last_sync: 1 }],
      };
      case "contact_details": return [{ id: "d1", source: "directory", account_email: "me@corp.com", folder: null, name: "Ana Diaz", emails: ["ana@corp.com"], phones: [],
        job_title: "CFO", department: null, office: null, company: null, favourite: false }];
      case "list_calendars": return [{ id: "cal1", name: "Calendar", color: "#0a84ff", is_primary: true, can_edit: true, account_email: "me@corp.com", provider: "microsoft" }];
      case "pending_actions": return 0;
      case "sync_now": return { folders: 0, messages_changed: 0, bodies_downloaded: 0, replayed: 0, events: 0 };
      case "search_messages": return [];
      default: return /^(list|pending|search)/.test(String(cmd)) ? [] : null;
    }
  });
}
beforeEach(() => mocked.mockReset());
afterEach(cleanup);

describe("Contacts tab", () => {
  it("opens from the ribbon's View tab next to Mail and Calendar, and from the sidebar", async () => {
    backend([{ email: "me@corp.com", display_name: "Me", provider: "microsoft" }]);
    render(<Mailbox account={{ email: "me@corp.com", display_name: "Me" }} onSignOut={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "View" }));
    const show = screen.getByRole("button", { name: "Mail" }).parentElement!;
    expect(within(show).getAllByRole("button").map((b) => b.textContent)).toEqual(["Mail", "Calendar", "Contacts"]);
    fireEvent.click(within(show).getByRole("button", { name: "Contacts" }));
    expect(await screen.findByRole("group", { name: "Directory" })).toBeTruthy();
    expect(await screen.findByText("Ana Diaz")).toBeTruthy();
    expect(screen.getByRole("region", { name: "Contact details" })).toBeTruthy();
    // Back to mail, then to contacts again through the sidebar.
    fireEvent.click(within(show).getByRole("button", { name: "Mail" }));
    await waitFor(() => expect(screen.queryByRole("group", { name: "Directory" })).toBeNull());
    fireEvent.click(screen.getByText("Contacts", { selector: ".sidebar .name" }));
    expect(await screen.findByRole("group", { name: "Directory" })).toBeTruthy();
  });

  it("explains that contacts need Microsoft 365 when only Gmail accounts are connected", async () => {
    backend([{ email: "me@gmail.com", display_name: "Gm", provider: "imap" }]);
    render(<Mailbox account={{ email: "me@gmail.com", display_name: "Gm" }} onSignOut={() => {}} />);
    await waitFor(() => expect(mocked).toHaveBeenCalledWith("list_accounts"));
    fireEvent.click(screen.getByText("Contacts", { selector: ".sidebar .name" }));
    expect(await screen.findByText("Contacts are available for Microsoft 365 accounts.")).toBeTruthy();
  });

  it("searches mail with a contact and starts a meeting with them from the right-click menu", async () => {
    backend([{ email: "me@corp.com", display_name: "Me", provider: "microsoft" }]);
    render(<Mailbox account={{ email: "me@corp.com", display_name: "Me" }} onSignOut={() => {}} />);
    fireEvent.click(screen.getByText("Contacts", { selector: ".sidebar .name" }));
    fireEvent.contextMenu(await screen.findByText("Ana Diaz"));
    fireEvent.click(await screen.findByRole("menuitem", { name: /Search mail with this person/ }));
    await waitFor(() => expect(mocked).toHaveBeenCalledWith("search_messages", { query: "with:ana@corp.com" }));
    expect((screen.getByPlaceholderText("Search") as HTMLInputElement).value).toBe("with:ana@corp.com");
    expect(screen.getByText("Mail with ana@corp.com")).toBeTruthy();

    fireEvent.click(screen.getByText("Contacts", { selector: ".sidebar .name" }));
    fireEvent.contextMenu(await screen.findByText("Ana Diaz"));
    fireEvent.click(await screen.findByRole("menuitem", { name: /New meeting/ }));
    expect(await screen.findByDisplayValue("ana@corp.com")).toBeTruthy(); // the invitee is filled in
  });
});

describe("Mail list right-click menu", () => {
  const inbox = { id: "f1", remote_id: "r1", name: "Inbox", well_known: "inbox", unread: 1, total: 1, parent_remote_id: null, account_email: "me@corp.com" };
  const msg = { id: "m1", subject: "Hello", sender_name: "Ana", sender_address: "ana@corp.com", snippet: "hi", received_at: 1, is_read: true, is_flagged: false, has_attachments: false, account_email: "me@corp.com" };
  const openMenu = async () => {
    render(<Mailbox account={{ email: "me@corp.com", display_name: "Me" }} onSignOut={() => {}} />);
    fireEvent.contextMenu(await screen.findByText("Hello"));
    return screen.getByRole("menu", { name: "Message actions" });
  };
  const calledWith = (cmd: string) => mocked.mock.calls.filter(([c]) => c === cmd);

  it("never starts the keyboard on Delete, even while Reply is unavailable, and a first Enter can't delete", async () => {
    // get_message never answers: Reply / Reply All / Forward stay disabled, so the first enabled item is Delete.
    backend([{ email: "me@corp.com", display_name: "Me", provider: "microsoft" }], { list_folders: () => [inbox], list_messages: () => [msg], get_message: () => new Promise(() => {}) });
    const menu = await openMenu();
    const focused = document.activeElement as HTMLElement;
    expect(menu.contains(focused)).toBe(true);
    expect(focused.textContent).not.toMatch(/Delete|spam|Block/);
    expect(focused.textContent).toContain("Move to…");
    fireEvent.keyDown(menu, { key: "Enter" });
    expect(calledWith("delete_message")).toHaveLength(0);
    expect(calledWith("mark_spam")).toHaveLength(0);
    expect(screen.getByPlaceholderText("Find a folder")).toBeTruthy(); // Move to… opened the folder picker instead
  });

  it("starts on Reply once the message is open, and Delete still works when chosen on purpose", async () => {
    backend([{ email: "me@corp.com", display_name: "Me", provider: "microsoft" }], { list_folders: () => [inbox], list_messages: () => [msg], get_message: () => ({ ...msg, to_json: null, cc_json: null, body_html: "" }) });
    render(<Mailbox account={{ email: "me@corp.com", display_name: "Me" }} onSignOut={() => {}} />);
    const row = await screen.findByText("Hello", { selector: ".subj" });
    fireEvent.click(row);
    await waitFor(() => expect(screen.getAllByRole("button", { name: /^Reply/ }).some((b) => !(b as HTMLButtonElement).disabled)).toBe(true)); // the message is open
    fireEvent.contextMenu(row);
    const menu = screen.getByRole("menu", { name: "Message actions" });
    expect(document.activeElement?.textContent).toContain("Reply");
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toContain("Delete");
    fireEvent.keyDown(menu, { key: "Enter" });
    await waitFor(() => expect(calledWith("delete_message")).toHaveLength(1));
  });
});
