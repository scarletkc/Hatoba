import { api } from "@/ipc/api";

/** Path suggestions on an SSH tab (TERM-16): the files and folders of a path being typed, through SFTP. */

/** What a suggestion needs of a listed file or folder. */
export interface PathEntry {
  name: string;
  is_dir: boolean;
}

/** Folders a tab keeps the listing of. */
const MAX_FOLDERS = 20;
/** A listing older than this is read again when it is used. */
const FRESH_MS = 10_000;

/** Whether `typed` ends inside a quote the shell has yet to see closed. */
function endsInQuote(typed: string): boolean {
  let quote: string | null = null;
  for (let i = 0; i < typed.length; i++) {
    const c = typed[i];
    if (c === "\\" && quote !== "'") i++;
    else if (quote !== null) quote = c === quote ? null : quote;
    else if (c === "'" || c === '"') quote = c;
  }
  return quote !== null;
}

/**
 * The folder and the start of a name that the last word of `typed` holds, when it is a plain path that
 * starts with `/` or `~/` and has part of a name after its last `/`. A word inside quotes, or with a
 * variable or a wildcard, is left to the shell.
 */
export function pathWord(typed: string): { folder: string; prefix: string } | null {
  if (endsInQuote(typed)) return null;
  const word = /(?:^|\s)((?:[^\s\\]|\\.)*)$/u.exec(typed)?.[1];
  if (!word || !/^~?\//u.test(word)) return null;
  if (/[$`*?"'[\]{}();&|<>]/u.test(word.replace(/\\./gu, ""))) return null;
  const path = word.replace(/\\(.)/gu, "$1");
  const end = path.lastIndexOf("/") + 1;
  return end === path.length ? null : { folder: path.slice(0, end), prefix: path.slice(end) };
}

/**
 * What continues `prefix` to the shortest name that starts with it, escaped for the shell, with a `/`
 * after a folder; or null. The whole name of a file has nothing to add.
 */
export function pathRest(prefix: string, entries: readonly PathEntry[]): string | null {
  let best: PathEntry | null = null;
  for (const entry of entries) {
    if (!entry.name.startsWith(prefix) || /\p{Cc}/u.test(entry.name)) continue;
    if (best === null || entry.name.length < best.name.length) best = entry;
  }
  if (best === null) return null;
  const rest = best.name.slice(prefix.length) + (best.is_dir ? "/" : "");
  return rest === "" ? null : rest.replace(/[^\p{L}\p{N}_@%+=:,./~-]/gu, "\\$&");
}

/** The listings of one tab's connection, read as a path is typed. */
export class PathLister {
  private sid: string | null = null;
  private home: string | null = null;
  /** Asked once per connection: a server that refuses SFTP or times out gets nothing more on it. */
  private homeAsked = false;
  private readonly folders = new Map<string, { entries: readonly PathEntry[]; at: number }>();
  private readonly loading = new Set<string>();

  /** `onChange` runs when an answer may change the suggestion. */
  constructor(private readonly onChange: () => void) {}

  /** The tab's SSH session changed; null when it has none. */
  connected(sid: string | null) {
    this.sid = sid;
    this.home = null;
    this.homeAsked = false;
    this.folders.clear();
    this.loading.clear();
  }

  /** What continues the path `typed` ends with, as far as it is listed; starts listing what is missing. */
  rest(typed: string): string | null {
    if (this.sid === null) return null;
    const word = pathWord(typed);
    if (!word) return null;
    this.askHome(this.sid);
    if (this.home === null) return null;
    const folder = word.folder.startsWith("~/") ? this.home.replace(/\/$/u, "") + word.folder.slice(1) : word.folder;
    const known = this.folders.get(folder);
    if (!known || Date.now() - known.at > FRESH_MS) this.list(this.sid, folder);
    return known ? pathRest(word.prefix, known.entries) : null;
  }

  /** The first SFTP request of a connection: without an answer, nothing is listed on it. */
  private askHome(sid: string) {
    if (this.homeAsked) return;
    this.homeAsked = true;
    api.sftp_home(sid).then(
      (home) => {
        if (sid !== this.sid) return;
        this.home = home;
        this.onChange();
      },
      () => {},
    );
  }

  private list(sid: string, folder: string) {
    if (this.loading.has(folder)) return;
    this.loading.add(folder);
    const done = (entries: readonly PathEntry[]) => {
      if (sid !== this.sid) return;
      this.loading.delete(folder);
      this.folders.delete(folder);
      if (this.folders.size >= MAX_FOLDERS) this.folders.delete(this.folders.keys().next().value!);
      this.folders.set(folder, { entries, at: Date.now() });
      this.onChange();
    };
    // A folder that cannot be read has nothing to suggest.
    api.sftp_list(sid, folder.length > 1 ? folder.replace(/\/$/u, "") : folder).then(done, () => done([]));
  }
}
