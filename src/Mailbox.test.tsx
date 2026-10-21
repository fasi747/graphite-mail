// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Mailbox from "./Mailbox";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));
const mocked = vi.mocked(invoke);

beforeAll(() => {
  globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as any;
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} })) as any;
});

function backend(accounts: object[]) {
  mocked.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_accounts": return accounts;
      case "get_theme": return "white";
      case "contacts_overview": return {
        directory: { total: 1, rows: [{ id: "d1", source: "directory", account_email: "me@corp.com", name: "Ana Diaz", email: "ana@corp.com", subtitle: "CFO" }] },
        personal: { total: 0, rows: [] },
        accounts: [{ email: "me@corp.com", granted: true, directory: "ok", personal: "ok", directory_count: 1, personal_count: 0, last_sync: 1 }],
      };
      case "pending_actions": return 0;
      case "sync_now": return { folders: 0, messages_changed: 0, bodies_downloaded: 0, replayed: 0, events: 0 };
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
});
