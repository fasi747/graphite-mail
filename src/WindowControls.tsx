import { getCurrentWindow } from "@tauri-apps/api/window";

const win = () => {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
};

/** Window buttons for the frameless window, top-right like Ubuntu: minimise, maximise, close. */
export function TrafficLights() {
  return (
    <div className="traffic">
      <span className="min" title="Minimise" onClick={() => win()?.minimize()}>−</span>
      <span className="max" title="Maximise" onClick={() => win()?.toggleMaximize()}>+</span>
      <span className="close" title="Close" onClick={() => win()?.close()}>✕</span>
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
