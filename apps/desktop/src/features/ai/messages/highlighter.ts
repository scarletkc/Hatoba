/* The grammars of `highlight.ts`, in a chunk of their own that loads with the first block that needs it. */

import apache from "highlight.js/lib/languages/apache";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import dos from "highlight.js/lib/languages/dos";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import lua from "highlight.js/lib/languages/lua";
import makefile from "highlight.js/lib/languages/makefile";
import markdown from "highlight.js/lib/languages/markdown";
import nginx from "highlight.js/lib/languages/nginx";
import php from "highlight.js/lib/languages/php";
import powershell from "highlight.js/lib/languages/powershell";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import shell from "highlight.js/lib/languages/shell";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import { createLowlight } from "lowlight";
import type { Token } from "./highlight";

const lowlight = createLowlight({
  apache,
  bash,
  c,
  cpp,
  css,
  diff,
  dockerfile,
  dos,
  go,
  ini,
  java,
  javascript,
  json,
  lua,
  makefile,
  markdown,
  nginx,
  php,
  powershell,
  python,
  ruby,
  rust,
  shell,
  sql,
  typescript,
  xml,
  yaml,
});

/** Whether `grammar` is registered. */
export const registered = (grammar: string) => lowlight.registered(grammar);

/** `code` as tokens of `grammar`, which must be registered. */
export function highlight(grammar: string, code: string): Token[] {
  return lowlight.highlight(grammar, code).children;
}
