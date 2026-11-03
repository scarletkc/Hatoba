import { useDeferredValue, useEffect, useMemo, useState } from "react";

/** A node of the highlighted code: text, or a span with the token's `hljs-*` classes. */
export interface Token {
  type: string;
  value?: string;
  properties?: { className?: unknown };
  children?: Token[];
}

/**
 * The fence languages that get colors, by the grammar that colors them: what people paste around
 * servers. Any other language, and a block without one, stays plain. `shell` is a script, as on
 * GitHub; `console` is a session with prompts and output.
 */
const GRAMMARS: Record<string, string[]> = {
  bash: ["bash", "sh", "shell", "zsh", "ksh", "dotenv", "env"],
  shell: ["console", "shellsession"],
  powershell: ["powershell", "pwsh", "ps1"],
  dos: ["bat", "batch", "cmd", "dos"],
  json: ["json", "jsonc", "json5"],
  yaml: ["yaml", "yml"],
  ini: ["ini", "toml", "systemd", "properties"],
  python: ["python", "py", "python3"],
  javascript: ["javascript", "js", "jsx", "mjs", "cjs"],
  typescript: ["typescript", "ts", "tsx", "mts", "cts"],
  go: ["go", "golang"],
  rust: ["rust", "rs"],
  sql: ["sql", "mysql", "postgresql", "postgres", "pgsql", "psql", "sqlite", "plsql"],
  dockerfile: ["dockerfile", "docker", "containerfile"],
  nginx: ["nginx", "nginxconf"],
  apache: ["apache", "apacheconf", "htaccess"],
  xml: ["xml", "html", "htm", "xhtml", "svg", "plist"],
  css: ["css"],
  markdown: ["markdown", "md"],
  diff: ["diff", "patch"],
  makefile: ["makefile", "make", "mk"],
  c: ["c", "h"],
  cpp: ["cpp", "c++", "cc", "cxx", "hpp"],
  java: ["java"],
  php: ["php"],
  ruby: ["ruby", "rb"],
  lua: ["lua"],
};

const BY_NAME = new Map(Object.entries(GRAMMARS).flatMap(([grammar, names]) => names.map((n) => [n, grammar])));

/** The grammar names the highlighter registers. */
export const GRAMMAR_NAMES = Object.keys(GRAMMARS);

/** The grammar for a fenced block's `language-*` class, or null to leave it plain. */
export function grammarOf(className: unknown): string | null {
  const classes = Array.isArray(className) ? className : typeof className === "string" ? className.split(" ") : [];
  for (const c of classes) {
    if (typeof c === "string" && c.startsWith("language-"))
      return BY_NAME.get(c.slice("language-".length).toLowerCase()) ?? null;
  }
  return null;
}

/** Longer blocks stay plain, so a long paste never holds up the panel. */
const MAX_CHARS = 30_000;

type Highlighter = typeof import("./highlighter");
let loaded: Highlighter | null = null;
let loading: Promise<Highlighter> | null = null;

/** The grammars load on the first block that needs them, not at startup. */
function load(): Promise<Highlighter> {
  loading ??= import("./highlighter").then(
    (m) => (loaded = m),
    (e: unknown) => {
      loading = null;
      throw e;
    },
  );
  return loading;
}

/**
 * The tokens of `code` in `grammar`, or null while the grammars load and for plain blocks. While a
 * reply streams, a block's new text is highlighted at a lower priority than the text arriving, and
 * shows plain in `rest` until its tokens catch up; blocks whose text is unchanged keep their tokens.
 */
export function useHighlight(grammar: string | null, code: string): { tokens: Token[]; rest: string } | null {
  const wanted = grammar !== null && code.length <= MAX_CHARS;
  const [ready, setReady] = useState(() => loaded !== null);
  useEffect(() => {
    if (!wanted || ready) return;
    let live = true;
    load().then(
      () => live && setReady(true),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [wanted, ready]);
  const deferred = useDeferredValue(code);
  const tokens = useMemo(() => {
    if (!wanted || grammar === null || !ready || !loaded) return null;
    try {
      return loaded.highlight(grammar, deferred);
    } catch {
      return null;
    }
  }, [wanted, ready, grammar, deferred]);
  if (!tokens || !code.startsWith(deferred)) return null;
  return { tokens, rest: code.slice(deferred.length) };
}
