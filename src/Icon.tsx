// Minimal SF Symbols–style line icons (stroke-based, inherit currentColor).
const P: Record<string, string> = {
  inbox: "M3 13h5l1.5 2.5h5L16 13h5M5 5h14l2 8v6H3v-6z",
  drafts: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  sentitems: "M4 12l16-8-6 16-2.5-6.5z M11.5 13.5L20 4",
  archive: "M3 5h18v4H3zM5 9v10h14V9M10 13h4",
  deleteditems: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6",
  junkemail: "M12 3l9 16H3zM12 10v4M12 17v.5",
  outbox: "M3 13h5l1.5 2.5h5L16 13h5M5 5h14l2 8v6H3v-6zM12 12V3M9 6l3-3 3 3",
  folder: "M3 6h6l2 2h10v11H3z",
  refresh: "M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5",
  signout: "M15 4h4v16h-4M10 8l-4 4 4 4M6 12h10",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  moon: "M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z",
  appearance: "M12 3a9 9 0 1 0 0 18zM12 3a9 9 0 0 1 0 18",
  paperclip: "M20 11l-8.5 8.5a5 5 0 0 1-7-7L13 4a3.5 3.5 0 0 1 5 5l-8.5 8.5a2 2 0 0 1-3-3L14 7",
  flag: "M5 21V4M5 4h11l-2 4 2 4H5",
  compose: "M12 4H5v15h15v-7M18 3l3 3-9 9H9v-3z",
  reply: "M10 8L4 13l6 5M4 13h10a6 6 0 0 1 6 6",
  replyall: "M13 8l-6 5 6 5M8 8l-6 5 6 5M7 13h7a6 6 0 0 1 6 6",
  forward: "M14 8l6 5-6 5M20 13H10a6 6 0 0 0-6 6",
  trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
  move: "M3 6h6l2 2h10v11H3zM10 13h6M13 10l3 3-3 3",
  calendar: "M4 6h16v14H4zM4 10h16M8 3v5M16 3v5",
  unread: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z",
};

export default function Icon({ name, size = 16 }: { name: string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={P[name] ?? P.folder} />
    </svg>
  );
}
