import type { FileEntry } from "@/ipc/types";

/** A file list's glyph: a Phosphor icon name, filled or not, and its color. */
export interface FileIcon {
  name: string;
  fill: boolean;
  color: string;
}

/**
 * The kinds a file list tells apart, as Finder and Termius do, each with a page-shaped glyph of the bundled Phosphor
 * font so the column keeps one outline. Glyphs that print a type's letters (`file-pdf`, `file-sql`) stay to the types
 * they name.
 */
const KINDS: [glyph: string, extensions: string][] = [
  ["file-code", "c cc cjs clj cpp cs css cxx dart erl ex exs go gradle groovy h hpp hs htm html java js jsx kt kts"],
  ["file-code", "less lua m mjs php pl pm proto py r rb rs sass scala scss svelte swift ts tsx vue zig"],
  ["file-code", "bash bat cmd command csh fish ksh ps1 psm1 sh zsh"],
  ["file-code", "cfg cnf conf env hcl ini json json5 jsonc lock mount plist properties service socket target tf"],
  ["file-code", "tfvars timer toml xml yaml yml"],
  ["file-archive", "7z apk bz2 deb dmg gz img iso jar lz lz4 lzma rar rpm tar tbz tbz2 tgz txz war whl xz zip zst"],
  ["file-image", "avif bmp gif heic heif ico jpeg jpg png psd svg tif tiff webp"],
  ["file-video", "avi flv m4v mkv mov mp4 mpeg mpg webm wmv"],
  ["file-audio", "aac aiff flac m4a mp3 oga ogg opus wav wma"],
  ["file-pdf", "pdf"],
  ["file-sql", "db db3 mdb rdb sql sqlite sqlite3"],
  ["file-doc", "doc docx epub odt pages rtf"],
  ["file-xls", "numbers ods xls xlsx"],
  ["file-csv", "csv tsv"],
  ["file-ppt", "odp ppt pptx"],
  ["file-text", "adoc cer crt csr der diff err log markdown md nfo out patch pem pub rst text txt"],
  ["file-lock", "jks key p12 pfx ppk"],
];

const BY_EXTENSION = new Map(
  KINDS.flatMap(([glyph, extensions]) => extensions.split(" ").map((ext) => [ext, glyph] as const)),
);

/** Files known by their whole name, lowercased. */
const BY_NAME = new Map<string, string>([
  ["dockerfile", "file-code"],
  ["containerfile", "file-code"],
  ["makefile", "file-code"],
  ["gemfile", "file-code"],
  ["rakefile", "file-code"],
  ["vagrantfile", "file-code"],
  ["jenkinsfile", "file-code"],
  ["crontab", "file-code"],
  ["authorized_keys", "file-text"],
  ["known_hosts", "file-text"],
]);

/** Text that a project or a package ships under a capital name without an extension. */
const TEXT_NAME =
  /^(readme|license|licence|changelog|changes|authors|contributors|contributing|copying|notice|todo)\b/i;
/** A private key as `ssh-keygen` names it (its `.pub` goes by the extension). */
const SSH_KEY = /^id_(rsa|dsa|ecdsa|ed25519)(_sk)?$/;
/** A dotfile that holds settings: `.bashrc`, `.profile`, `.gitconfig`, `.gitignore`, `.env.production`. */
const DOT_CONFIG = /^\.(.*rc|.*profile|.*config|.*ignore|gitattributes|gitmodules|editorconfig|env(\..+)?|tmux\.conf)$/;
/** A rotated log: `syslog.1`, `app.log.2`. */
const ROTATED_LOG = /\.log\.\d+$|^(syslog|messages|auth\.log|kern\.log)(\.\d+)?$/;

/**
 * The glyph of a file list's entry: a folder; a lock for an owner-only file or a private key; else the file's kind by
 * its extension or name; else a blank page. Only folders take the accent color.
 */
export function fileIcon(e: Pick<FileEntry, "name" | "is_dir" | "permissions">): FileIcon {
  if (e.is_dir) return { name: "folder-simple", fill: true, color: "var(--accent)" };
  return { name: fileGlyph(e), fill: false, color: "var(--fg2)" };
}

function fileGlyph({ name, permissions }: Pick<FileEntry, "name" | "permissions">): string {
  if (/^-rw-------/.test(permissions)) return "file-lock";
  const lower = name.toLowerCase();
  const byName = BY_NAME.get(lower);
  if (byName) return byName;
  if (SSH_KEY.test(lower)) return "file-lock";
  if (DOT_CONFIG.test(lower)) return "file-code";
  if (ROTATED_LOG.test(lower)) return "file-text";
  const dot = lower.lastIndexOf(".");
  const byExtension = dot > 0 ? BY_EXTENSION.get(lower.slice(dot + 1)) : undefined;
  if (byExtension) return byExtension;
  if (TEXT_NAME.test(name)) return "file-text";
  return "file";
}
