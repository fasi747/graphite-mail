// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ContactsView, { ContactDetail, ContactRow, ContactStatus, Overview, statusNote } from "./ContactsView";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);

const ms = { email: "me@corp.com", display_name: "Me", provider: "microsoft" };
const ms2 = { email: "you@other.com", display_name: "You", provider: "microsoft" };
const gmail = { email: "me@gmail.com", display_name: "Gm", provider: "imap" };

const row = (id: string, name: string, source: ContactRow["source"], email: string, subtitle: string | null = null): ContactRow =>
  ({ id, source, account_email: "me@corp.com", name, email, subtitle });
const ROWS = [
  row("d1", "Ana Diaz", "directory", "ana@corp.com", "CFO"),
  row("d2", "Ben Ode", "directory", "ben@corp.com"),
  row("p1", "Anna Bell", "personal", "anna@home.com"),
];
const ok: ContactStatus = { email: "me@corp.com", granted: true, directory: "ok", personal: "ok", directory_count: 2, personal_count: 1, last_sync: 1 };
const DETAIL: ContactDetail = {
  id: "d1", source: "directory", account_email: "me@corp.com", folder: null, name: "Ana Diaz", emails: ["ana@corp.com", "a.diaz@corp.com"],
  phones: [{ kind: "work", number: "+1 555 0100" }], job_title: "CFO", department: "Finance", office: "HQ 4", company: "Contoso",
};

/** A pretend backend: filters ROWS like the real search does, and records calls. */
function backend(status: ContactStatus = ok) {
  mocked.mockImplementation(async (cmd: string, args?: any) => {
    if (cmd === "contacts_overview") {
      const words = String(args.query).toLowerCase().split(/\s+/).filter(Boolean);
      const hit = (r: ContactRow) => words.every((w) => `${r.name} ${r.email} ${r.subtitle ?? ""}`.toLowerCase().includes(w));
      const of = (s: string) => { const rows = ROWS.filter((r) => r.source === s && hit(r)); return { total: rows.length, rows }; };
      return { directory: of("directory"), personal: of("personal"), accounts: [status] } satisfies Overview;
    }
    if (cmd === "contact_detail") return DETAIL;
    if (cmd === "refresh_contacts" || cmd === "grant_contacts") return [ok];
    return null;
  });
}
const calls = (cmd: string) => mocked.mock.calls.filter(([c]) => c === cmd);

beforeEach(() => { mocked.mockReset(); });
afterEach(cleanup);

describe("Contacts view", () => {
  it("shows the directory and my contacts as groups with counts, and folds them", async () => {
    backend();
    render(<ContactsView accounts={[ms, gmail]} onNewEmail={() => {}} onToast={() => {}} />);
    const dir = await screen.findByRole("group", { name: "Directory" });
    await waitFor(() => expect(within(dir).getByText("Ana Diaz")).toBeTruthy());
    expect(within(dir).getByRole("button", { name: /Directory\s*2/ })).toBeTruthy();
    expect(within(dir).getByText("CFO")).toBeTruthy();
    const mine = screen.getByRole("group", { name: "My contacts" });
    expect(within(mine).getByRole("button", { name: /My contacts\s*1/ })).toBeTruthy();
    expect(within(mine).getByText("Anna Bell")).toBeTruthy();
    // Folding a group hides its people.
    const head = within(dir).getByRole("button", { name: /Directory/ });
    fireEvent.click(head);
    expect(head.getAttribute("aria-expanded")).toBe("false");
    expect(within(dir).queryByText("Ana Diaz")).toBeNull();
    fireEvent.click(head);
    expect(within(dir).getByText("Ana Diaz")).toBeTruthy();
    expect(screen.queryByRole("combobox", { name: "Account" })).toBeNull(); // one Microsoft account: no account filter
  });

  it("searches both groups with one box", async () => {
    backend();
    render(<ContactsView accounts={[ms]} onNewEmail={() => {}} onToast={() => {}} />);
    await screen.findByText("Ben Ode");
    fireEvent.change(screen.getByRole("searchbox", { name: "Search contacts" }), { target: { value: "an" } });
    await waitFor(() => expect(screen.queryByText("Ben Ode")).toBeNull());
    expect(screen.getByText("Ana Diaz")).toBeTruthy();
    expect(screen.getByText("Anna Bell")).toBeTruthy();
    expect(calls("contacts_overview").slice(-1)[0][1]).toEqual({ query: "an", account: null });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search contacts" }), { target: { value: "zzz" } });
    await waitFor(() => expect(screen.getAllByText("No matches")).toHaveLength(2));
  });

  it("shows the selected person, writes to them and copies their address", async () => {
    backend();
    const onNewEmail = vi.fn();
    const onToast = vi.fn();
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    render(<ContactsView accounts={[ms]} onNewEmail={onNewEmail} onToast={onToast} />);
    fireEvent.click(await screen.findByText("Ana Diaz"));
    const pane = screen.getByRole("region", { name: "Contact details" });
    await waitFor(() => expect(within(pane).getByRole("heading", { name: "Ana Diaz" })).toBeTruthy());
    for (const t of ["ana@corp.com", "a.diaz@corp.com", "+1 555 0100", "HQ 4", "Contoso", "Finance"]) expect(within(pane).getAllByText(new RegExp(t.replace(/[+.]/g, "\\$&"))).length).toBeGreaterThan(0);
    fireEvent.click(within(pane).getByRole("button", { name: /New email/ }));
    expect(onNewEmail).toHaveBeenCalledWith("ana@corp.com", "me@corp.com");
    await act(async () => { fireEvent.click(within(pane).getByRole("button", { name: /Copy email/ })); });
    expect(writeText).toHaveBeenCalledWith("ana@corp.com");
    expect(onToast).toHaveBeenCalledWith("Email address copied");
  });

  it("explains missing permission and offers Grant access", async () => {
    backend({ ...ok, granted: false, directory: "needs_consent", personal: "needs_consent", directory_count: 0, personal_count: 0 });
    render(<ContactsView accounts={[ms]} onNewEmail={() => {}} onToast={() => {}} />);
    expect(await screen.findByText(/needs your permission to read contacts/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Grant access" }));
    await waitFor(() => expect(calls("grant_contacts")[0][1]).toEqual({ email: "me@corp.com" }));
  });

  it("refreshes on request and filters by account when several are connected", async () => {
    backend();
    render(<ContactsView accounts={[ms, ms2]} onNewEmail={() => {}} onToast={() => {}} />);
    await screen.findByText("Ana Diaz");
    fireEvent.click(screen.getByRole("button", { name: "Refresh contacts" }));
    await waitFor(() => expect(calls("refresh_contacts").length).toBe(1));
    fireEvent.change(screen.getByRole("combobox", { name: "Account" }), { target: { value: "you@other.com" } });
    await waitFor(() => expect(calls("contacts_overview").slice(-1)[0][1]).toEqual({ query: "", account: "you@other.com" }));
  });

  it("downloads at once the first time", async () => {
    backend({ ...ok, directory: "never", personal: "never" });
    render(<ContactsView accounts={[ms]} onNewEmail={() => {}} onToast={() => {}} />);
    await waitFor(() => expect(calls("refresh_contacts").length).toBe(1));
  });

  it("says contacts need a Microsoft 365 account", () => {
    render(<ContactsView accounts={[gmail]} onNewEmail={() => {}} onToast={() => {}} />);
    expect(screen.getByText("Contacts are available for Microsoft 365 accounts.")).toBeTruthy();
    expect(mocked).not.toHaveBeenCalled();
  });

  it("words each problem", () => {
    expect(statusNote(ok)).toBeNull();
    expect(statusNote({ ...ok, directory: "blocked" })?.text).toMatch(/doesn't allow Graphite Mail to read the directory/);
    expect(statusNote({ ...ok, directory: "blocked" })?.grant).toBe(true);
    expect(statusNote({ ...ok, personal: "removed", directory: "removed" })?.grant).toBe(false);
    expect(statusNote({ ...ok, personal: "error" })?.text).toMatch(/offline/);
  });
});
