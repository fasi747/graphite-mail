// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(), open: vi.fn() }));

afterEach(cleanup);

function firstStart(googleOk: boolean) {
  vi.mocked(invoke).mockImplementation(async (cmd: string) => {
    if (cmd === "setup_state") return { ready: true, default_dir: "/data" };
    if (cmd === "google_sign_in_available") return googleOk;
    return null; // no account yet
  });
  render(<App />);
}

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
