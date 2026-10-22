// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { SIGN_IN_TIMEOUT_MS } from "./accounts";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));
vi.mock("./Mailbox", () => ({ default: ({ account }: { account: { email: string } }) => <div>Mailbox for {account.email}</div> }));

afterEach(() => { cleanup(); vi.useRealTimers(); });

const mocked = vi.mocked(invoke);
function firstStart(googleOk: boolean, extra: Record<string, (args?: unknown) => unknown> = {}) {
  mocked.mockReset();
  mocked.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in extra) return extra[cmd](args);
    if (cmd === "setup_state") return { ready: true, default_dir: "/data" };
    if (cmd === "google_sign_in_available") return googleOk;
    return null; // no account yet
  });
  return render(<App />);
}
const off = (el: HTMLElement) => (el as HTMLButtonElement).disabled;
const calls = (cmd: string) => mocked.mock.calls.filter(([c]) => c === cmd).length;

describe("first start", () => {
  it("doesn't offer Google sign-in in builds without the Google client secret", async () => {
    firstStart(false);
    expect(await screen.findByText("Google sign-in coming soon. Use an app password for now.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Sign in with Google" })).toBeNull();
    expect(screen.getByRole("button", { name: "Use Gmail with an app password instead" })).toBeTruthy();
  });

  it("offers it when the build has it", async () => {
    firstStart(true);
    expect(await screen.findByRole("button", { name: "Sign in with Google" })).toBeTruthy();
    expect(screen.queryByText(/coming soon/)).toBeNull();
  });
});

describe("first-run browser sign-in", () => {
  it("can be cancelled: the backend listener is stopped, buttons come back, and a late error is ignored", async () => {
    let fail: (e: unknown) => void = () => {};
    firstStart(true, { sign_in: () => new Promise((_, reject) => { fail = reject; }) });
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Microsoft" }));
    expect(off(screen.getByRole("button", { name: "Waiting for browser sign-in…" }))).toBe(true);
    expect(off(screen.getByRole("button", { name: "Sign in with Google" }))).toBe(true);
    expect(screen.getByText(/Finish signing in in your web browser/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
    expect(calls("cancel_sign_in")).toBe(1);
    expect(screen.getByText("Sign-in cancelled.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Cancel sign-in" })).toBeNull();
    expect(off(screen.getByRole("button", { name: "Sign in with Microsoft" }))).toBe(false);
    expect(off(screen.getByRole("button", { name: "Sign in with Google" }))).toBe(false);
    // The cancelled attempt finishes in the background: no error, no "waiting" coming back.
    await act(async () => fail("Sign-in was cancelled."));
    expect(screen.queryByText("Sign-in was cancelled.")).toBeNull();
    expect(screen.queryByRole("button", { name: "Waiting for browser sign-in…" })).toBeNull();
    expect(screen.getByText("Sign-in cancelled.")).toBeTruthy();
  });

  it("a cancelled attempt that signs in late doesn't open the mailbox; a fresh attempt does", async () => {
    let finish: (a: unknown) => void = () => {};
    const attempts: ((a: unknown) => void)[] = [];
    firstStart(true, { add_google_account: () => new Promise((resolve) => { attempts.push(resolve); finish = resolve; }) });
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google" }));
    expect(screen.getByRole("button", { name: "Waiting for browser sign-in…" }).textContent).toContain("Waiting");
    fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
    const stale = finish;
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google" }));
    expect(attempts).toHaveLength(2);
    await act(async () => stale({ email: "old@gmail.com", display_name: "Old" }));
    expect(screen.queryByText(/Mailbox for/)).toBeNull();
    expect(screen.getByRole("button", { name: "Waiting for browser sign-in…" })).toBeTruthy(); // still waiting for the second
    expect(screen.queryByText("Sign-in cancelled.")).toBeNull();
    await act(async () => attempts[1]({ email: "new@gmail.com", display_name: "New" }));
    expect(screen.getByText("Mailbox for new@gmail.com")).toBeTruthy(); // the mailbox opened for the fresh attempt
  });

  it("gives up after two minutes and says so", async () => {
    firstStart(false, { sign_in: () => new Promise(() => {}) });
    await screen.findByRole("button", { name: "Sign in with Microsoft" });
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    act(() => { vi.advanceTimersByTime(SIGN_IN_TIMEOUT_MS - 1000); });
    expect(calls("cancel_sign_in")).toBe(0);
    expect(screen.getByRole("button", { name: "Cancel sign-in" })).toBeTruthy();
    act(() => { vi.advanceTimersByTime(1000); });
    expect(calls("cancel_sign_in")).toBe(1);
    expect(screen.getByText(/took too long/)).toBeTruthy();
    expect(off(screen.getByRole("button", { name: "Sign in with Microsoft" }))).toBe(false);
    expect(off(screen.getByRole("button", { name: "Use Gmail with an app password instead" }))).toBe(false);
  });

  it("cleans up when the screen goes away: no timeout fires later", async () => {
    const { unmount } = firstStart(false, { sign_in: () => new Promise(() => {}) });
    await screen.findByRole("button", { name: "Sign in with Microsoft" });
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    unmount();
    act(() => { vi.advanceTimersByTime(SIGN_IN_TIMEOUT_MS + 1000); });
    expect(calls("cancel_sign_in")).toBe(0);
  });

  it("the app-password path puts the sign-in note away", async () => {
    firstStart(false, { sign_in: () => new Promise(() => {}) });
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Microsoft" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
    expect(screen.getByText("Sign-in cancelled.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Use Gmail with an app password instead" }));
    expect(screen.getByRole("form", { name: "Gmail with an app password" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Sign-in cancelled.")).toBeNull();
  });
});
