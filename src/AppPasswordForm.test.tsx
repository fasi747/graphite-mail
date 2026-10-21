// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import AppPasswordForm from "./AppPasswordForm";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);

const off = (el: HTMLElement) => (el as HTMLButtonElement).disabled;

beforeEach(() => { mocked.mockReset(); Element.prototype.scrollIntoView = vi.fn(); });
afterEach(cleanup);

const type = (label: string, value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });
const connect = () => screen.getByRole("button", { name: "Connect" });
const hint = () => document.getElementById("apppw-hint")?.textContent ?? null;

describe("Gmail app-password form", () => {
  it("says why Connect is inactive until both fields are filled in", () => {
    render(<AppPasswordForm onAdded={() => {}} />);
    expect(off(connect())).toBe(true);
    expect(hint()).toBe("Enter both fields to continue");
    expect(connect().getAttribute("aria-describedby")).toBe("apppw-hint");

    type("Gmail address", "me@gmail.com");
    expect(off(connect())).toBe(true);
    expect(hint()).toBe("Enter both fields to continue");

    type("App password", "abcd efgh ijkl");
    expect(off(connect())).toBe(true);
    expect(hint()).toBe("12 of 16 letters");

    type("Gmail address", "me");
    type("App password", "abcd efgh ijkl mnop");
    expect(off(connect())).toBe(true);
    expect(hint()).toMatch(/full Gmail address/);

    type("Gmail address", "me@gmail.com");
    expect(off(connect())).toBe(false);
    expect(hint()).toBeNull();
    expect(connect().getAttribute("aria-describedby")).toBeNull();
  });

  it("sends the code without Google's spaces", async () => {
    const added = vi.fn();
    mocked.mockResolvedValue({ email: "me@gmail.com", display_name: "me@gmail.com", provider: "imap" });
    render(<AppPasswordForm onAdded={added} />);
    type("Gmail address", " me@gmail.com ");
    type("App password", "abcd efgh ijkl mnop");
    fireEvent.click(connect());
    await waitFor(() => expect(added).toHaveBeenCalled());
    expect(mocked).toHaveBeenCalledWith("add_imap_account", { email: "me@gmail.com", password: "abcdefghijklmnop" });
  });

  it("puts the cursor in the address field when it opens", () => {
    render(<AppPasswordForm onAdded={() => {}} />);
    expect(document.activeElement).toBe(screen.getByLabelText("Gmail address"));
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
  });
});
