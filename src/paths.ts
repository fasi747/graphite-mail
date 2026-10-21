/** Small path helpers for the few places the UI builds a path itself (the data folder picker). */

/** The separator a folder path already uses: `\\` for Windows paths, `/` otherwise. */
export const separatorOf = (dir: string) => (/^[A-Za-z]:[\\/]|^\\\\/.test(dir) || (dir.includes("\\") && !dir.includes("/")) ? "\\" : "/");

/** `dir/name` with the separator that fits `dir`, never doubling one that is already there. */
export function joinPath(dir: string, name: string) {
  const sep = separatorOf(dir);
  const base = dir.replace(/[\\/]+$/, "");
  return `${base}${sep}${name}`;
}

/** The folder the user picked, with the app's own sub-folder added unless it already ends with it. */
export function dataFolderFrom(picked: string, leaf = "graphite-mail") {
  const trimmed = picked.replace(/[\\/]+$/, "");
  const last = trimmed.split(/[\\/]/).pop();
  return last === leaf ? trimmed : joinPath(trimmed, leaf);
}
