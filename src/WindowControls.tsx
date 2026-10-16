import { getCurrentWindow } from "@tauri-apps/api/window";

const win = () => {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
};

/** macOS-style traffic lights for the frameless window. */
export function TrafficLights() {
  return (
    <div className="traffic">
      <span className="close" onClick={() => win()?.close()}>✕</span>
      <span className="min" onClick={() => win()?.minimize()}>−</span>
      <span className="max" onClick={() => win()?.toggleMaximize()}>+</span>
    </div>
  );
}

/** Draggable title-bar strip; double-click toggles maximize like macOS. */
export function DragBar({ children }: { children?: React.ReactNode }) {
  return (
    <div className="dragbar" data-tauri-drag-region onDoubleClick={() => win()?.toggleMaximize()}>
      {children}
    </div>
  );
}
