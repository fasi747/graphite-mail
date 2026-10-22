import { FolderRole, TILES } from "./folderIcons";

/** A flat coloured rounded square with a white stroke glyph. Decorative: the label next to it carries the meaning. */
export default function FolderTile({ role, size = 24, glyph = 15, radius = 7 }: { role: FolderRole; size?: number; glyph?: number; radius?: number }) {
  const t = TILES[role];
  return (
    <span className="tile" aria-hidden="true" data-role={role} style={{ width: size, height: size, borderRadius: radius, background: t.colour }}>
      <svg width={glyph} height={glyph} viewBox="0 0 24 24" fill="none" stroke="#ffffff" strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round">
        <path d={t.path} />
      </svg>
    </span>
  );
}
