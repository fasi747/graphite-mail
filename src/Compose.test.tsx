// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Compose from "./Compose";
import { lastToken, replaceLastToken } from "./recipients";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
const mocked = vi.mocked(invoke);

const people = [{ name: "Anna Bell", email: "anna@home.com" }, { name: "Ana Diaz", email: "ana@corp.com" }];

beforeEach(() => {
  mocked.mockReset();
  mocked.mockImplementation(async (cmd: string, args?: any) => {
    if (cmd === "suggest_recipients") return args.account === "me@corp.com" && args.query.length >= 2 ? people.filter((p) => `${p.name} ${p.email}`.toLowerCase().includes(args.query)) : [];
    if (cmd === "get_signature") return "";
    return null;
  });
});
afterEach(cleanup);

const open = () => render(<Compose init={{ mode: "new" }} accounts={[{ email: "me@corp.com", display_name: "Me" }]} onClose={() => {}} onSent={() => {}} />);

describe("To / Cc suggestions", () => {
  it("suggests contacts while typing and completes with the keyboard", async () => {
    open();
    const to = screen.getByRole("combobox", { name: "To" });
    fireEvent.change(to, { target: { value: "an" } });
    const list = await screen.findByRole("listbox", { name: "To suggestions" });
    expect(list.textContent).toContain("Anna Bell");
    expect(to.getAttribute("aria-expanded")).toBe("true");
    expect(mocked).toHaveBeenCalledWith("suggest_recipients", { query: "an", account: "me@corp.com" });
    fireEvent.keyDown(to, { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(to, { key: "Enter" });
    expect((to as HTMLInputElement).value).toBe("ana@corp.com, ");
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
  });

  it("works in Cc too, by clicking a suggestion, after addresses already typed", async () => {
    open();
    const cc = screen.getByRole("combobox", { name: "Cc" });
    fireEvent.change(cc, { target: { value: "bob@x.com, ann" } });
    const option = await screen.findByRole("option", { name: /Anna Bell/ });
    fireEvent.mouseDown(option);
    expect((cc as HTMLInputElement).value).toBe("bob@x.com, anna@home.com, ");
  });

  it("stays quiet for one letter and when nothing matches", async () => {
    open();
    const to = screen.getByRole("combobox", { name: "To" });
    fireEvent.change(to, { target: { value: "a" } });
    fireEvent.change(to, { target: { value: "zz" } });
    await waitFor(() => expect(mocked).toHaveBeenCalledWith("suggest_recipients", { query: "zz", account: "me@corp.com" }));
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(mocked.mock.calls.some(([c, a]: any) => c === "suggest_recipients" && a.query === "a")).toBe(false);
  });
});

describe("To / Cc suggestions (quiet cases)", () => {
  it("doesn't pop up for addresses that were already there when the message opened", async () => {
    render(<Compose init={{ mode: "new", to: ["ana@corp.com"] }} accounts={[{ email: "me@corp.com", display_name: "Me" }]} onClose={() => {}} onSent={() => {}} />);
    await new Promise((r) => setTimeout(r, 200));
    expect(mocked.mock.calls.some(([c]) => c === "suggest_recipients")).toBe(false);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("closes when the field loses focus", async () => {
    open();
    const to = screen.getByRole("combobox", { name: "To" });
    fireEvent.change(to, { target: { value: "an" } });
    await screen.findByRole("listbox", { name: "To suggestions" });
    fireEvent.blur(to);
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});

describe("recipient tokens", () => {
  it("finds and replaces the address being typed", () => {
    expect(lastToken("a@x.com, bo")).toBe("bo");
    expect(lastToken("a@x.com; bo ")).toBe("bo");
    expect(lastToken("bo")).toBe("bo");
    expect(replaceLastToken("bo", "bob@x.com")).toBe("bob@x.com, ");
    expect(replaceLastToken("a@x.com, bo", "bob@x.com")).toBe("a@x.com, bob@x.com, ");
    expect(replaceLastToken("a@x.com;bo", "bob@x.com")).toBe("a@x.com; bob@x.com, ");
  });
});
