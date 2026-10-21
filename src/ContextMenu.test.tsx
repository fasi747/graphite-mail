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

describe("destructive items and the first keypress", () => {
  const setupMail = (replyOn: boolean) => {
    const pick = vi.fn();
    const items: MenuItem[] = [
      { label: "Reply", disabled: !replyOn, onClick: () => pick("reply") },
      { label: "Delete", destructive: true, onClick: () => pick("delete"), separator: true },
      { label: "Move to…", onClick: () => pick("move") },
      { label: "Block sender", destructive: true, onClick: () => pick("block") },
      { label: "Mark as spam", destructive: true, onClick: () => pick("spam") },
    ];
    render(<ContextMenu x={10} y={10} items={items} onClose={() => {}} />);
    return { pick, root: screen.getByRole("menu", { name: "Message actions" }) };
  };

  it("never starts on Delete when the items before it are disabled; Enter picks the safe item", () => {
    const { pick, root } = setupMail(false);
    expect(focused()).toBe("Move to…");
    fireEvent.keyDown(root, { key: "Enter" });
    expect(pick).toHaveBeenCalledWith("move");
    expect(pick).not.toHaveBeenCalledWith("delete");
  });

  it("starts on the first ordinary item when there is one, and arrows still reach Delete on purpose", () => {
    const { pick, root } = setupMail(true);
    expect(focused()).toBe("Reply");
    fireEvent.keyDown(root, { key: "ArrowDown" });
    expect(focused()).toBe("Delete");
    fireEvent.keyDown(root, { key: "Enter" });
    expect(pick).toHaveBeenCalledWith("delete");
  });

  it("focuses the menu itself when every enabled item is destructive, so Enter does nothing until an item is chosen", () => {
    const pick = vi.fn();
    const items: MenuItem[] = [
      { label: "Reply", disabled: true, onClick: () => pick("reply") },
      { label: "Delete", destructive: true, onClick: () => pick("delete") },
      { label: "Mark as spam", destructive: true, onClick: () => pick("spam") },
    ];
    render(<ContextMenu x={10} y={10} items={items} onClose={() => {}} />);
    const root = screen.getByRole("menu", { name: "Message actions" });
    expect(document.activeElement).toBe(root);
    expect(root.querySelector(".focus")).toBeNull();
    fireEvent.keyDown(root, { key: "Enter" });
    fireEvent.keyDown(root, { key: " " });
    expect(pick).not.toHaveBeenCalled();
    fireEvent.keyDown(root, { key: "ArrowUp" }); // wraps to the last enabled item
    expect(focused()).toBe("Mark as spam");
    fireEvent.keyDown(root, { key: "ArrowDown" });
    expect(focused()).toBe("Delete");
    fireEvent.keyDown(root, { key: "Enter" });
    expect(pick).toHaveBeenCalledWith("delete");
  });
});
