// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenu, MenuItem } from "./ContextMenu";

afterEach(cleanup);

function setup() {
  const pick = vi.fn();
  const onClose = vi.fn();
  const items: MenuItem[] = [
    { label: "New email", submenu: [{ label: "a@x.com", onClick: () => pick("a") }, { label: "b@x.com", onClick: () => pick("b") }] },
    { label: "Disabled", disabled: true, onClick: () => pick("never") },
    { label: "Copy", onClick: () => pick("copy") },
  ];
  render(<ContextMenu x={10} y={10} items={items} onClose={onClose} label="Contact actions" />);
  return { pick, onClose, root: screen.getByRole("menu", { name: "Contact actions" }) };
}
const focused = () => document.activeElement?.textContent;

describe("context menu with submenus", () => {
  it("opens a submenu with the right arrow, chooses with Enter, and closes everything", () => {
    const { pick, onClose, root } = setup();
    expect(focused()).toContain("New email");
    fireEvent.keyDown(root, { key: "ArrowRight" });
    const sub = screen.getByRole("menu", { name: "New email" });
    expect(screen.getAllByRole("menuitem")[0].getAttribute("aria-expanded")).toBe("true");
    expect(focused()).toBe("a@x.com");
    fireEvent.keyDown(sub, { key: "ArrowDown" });
    expect(focused()).toBe("b@x.com");
    fireEvent.keyDown(sub, { key: "Enter" });
    expect(pick).toHaveBeenCalledWith("b");
    expect(onClose).toHaveBeenCalled();
  });

  it("goes back with the left arrow or Esc, skips disabled items, and Esc closes the menu", () => {
    const { pick, onClose, root } = setup();
    fireEvent.keyDown(root, { key: "Enter" }); // Enter on a submenu item opens it
    const sub = screen.getByRole("menu", { name: "New email" });
    fireEvent.keyDown(sub, { key: "ArrowLeft" });
    expect(screen.queryByRole("menu", { name: "New email" })).toBeNull();
    expect(focused()).toContain("New email");
    fireEvent.keyDown(root, { key: "ArrowRight" });
    fireEvent.keyDown(screen.getByRole("menu", { name: "New email" }), { key: "Escape" });
    expect(screen.queryByRole("menu", { name: "New email" })).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(root, { key: "ArrowDown" });
    expect(focused()).toContain("Copy"); // "Disabled" is skipped
    fireEvent.keyDown(root, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    expect(pick).not.toHaveBeenCalled();
  });

  it("opens a submenu on hover and runs a click", () => {
    const { pick } = setup();
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: /New email/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "a@x.com" }));
    expect(pick).toHaveBeenCalledWith("a");
  });
});
