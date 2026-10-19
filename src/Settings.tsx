import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";

type Info = { path: string; size_bytes: number };
type Progress = { phase: number; label: string; done: number; total: number };

const fmtSize = (b: number) => (b > 1 << 30 ? `${(b / (1 << 30)).toFixed(1)} GB` : `${Math.max(1, Math.round(b / (1 << 20)))} MB`);

export default function Settings({ onClose }: { onClose: () => void }) {
  const [info, setInfo] = useState<Info | null>(null);
  const [moving, setMoving] = useState<Progress | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  const load = () => invoke<Info>("data_folder_info").then(setInfo).catch((e) => setError(String(e)));
  useEffect(() => {
    load();
    const un = listen<Progress>("migration-progress", (e) => setMoving(e.payload));
    return () => { un.then((f) => f()); };
  }, []);

  async function move() {
    setError(""); setMessage("");
    const picked = await open({ directory: true, title: "Choose where Graphite Mail should keep your mail" });
    if (typeof picked !== "string") return;
    const target = picked.endsWith("graphite-mail") ? picked : `${picked}/graphite-mail`;
    const yes = await ask(
      `Move your mail and calendar data to:\n${target}\n\nThis can take a few minutes for a large mailbox. Your current data stays untouched until the copy has been checked.`,
      { title: "Move data folder", kind: "info", okLabel: "Move", cancelLabel: "Cancel" },
    );
    if (!yes) return;
    setMoving({ phase: 1, label: "Starting…", done: 0, total: 1 });
    try {
      const where = await invoke<string>("move_data_folder", { target });
      setMessage(`Done. Your data is now in ${where}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setMoving(null);
      load();
    }
  }

  const pct = moving ? Math.round(((moving.phase - 1 + (moving.total ? moving.done / moving.total : 0)) / 7) * 100) : 0;

  return (
    <div className="sheet-backdrop" onClick={() => !moving && onClose()}>
      <div className="sheet evsheet" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-bar"><span /><b>Settings</b><button disabled={!!moving} onClick={onClose}>Close</button></div>
        <div className="evbody">
          <div className="muted" style={{ fontSize: 11, fontWeight: 600 }}>DATA FOLDER</div>
          <div style={{ userSelect: "text", overflowWrap: "anywhere" }}>{info?.path ?? "…"}</div>
          {info && <div className="muted">{fmtSize(info.size_bytes)} used</div>}
          {moving ? (
            <div>
              <div>{moving.label} <span className="muted">(step {moving.phase} of 7)</span></div>
              <div className="progress"><div style={{ width: `${pct}%` }} /></div>
              <div className="muted" style={{ fontSize: 12 }}>Please keep Graphite Mail open. Your mail is safe even if the computer switches off.</div>
            </div>
          ) : (
            <div><button onClick={move}>Move to another folder…</button></div>
          )}
          {message && <div style={{ color: "#30a46c" }}>{message}</div>}
          {error && <div className="error">{error}</div>}
        </div>
      </div>
    </div>
  );
}
