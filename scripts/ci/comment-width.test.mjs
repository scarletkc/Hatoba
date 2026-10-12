import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_WIDTH, commentLines, longComments } from "./comment-width.mjs";

/** A line of exactly `width` characters that starts with `start`. */
const line = (start, width) => start + "x".repeat(width - [...start].length);

test("comment lines are measured in characters, not bytes", () => {
  const arrows = "    /// a → b → c ";
  assert.deepEqual(longComments(line(arrows, MAX_WIDTH)), []);
  assert.deepEqual(longComments(line(arrows, MAX_WIDTH + 1)), [{ line: 1, width: MAX_WIDTH + 1 }]);
  // Outside the Basic Multilingual Plane a character is two UTF-16 units.
  assert.deepEqual(longComments(line("// 🔑 ", MAX_WIDTH)), []);
});

test("every comment style is checked, at its line number", () => {
  const text = ["fn main() {}", line("//! ", 101), "", line("/// ", 102), line("        // ", 103), line("// ", 100)].join("\r\n");
  assert.deepEqual(longComments(text), [
    { line: 2, width: 101 },
    { line: 4, width: 102 },
    { line: 5, width: 103 },
  ]);
});

test("a comment that trails code is checked", () => {
  assert.deepEqual(longComments(line("    let mask = 0xffc0; // ", 101)), [{ line: 1, width: 101 }]);
});

test("long code lines without a comment are left to rustfmt", () => {
  assert.deepEqual(longComments(line('    let url = "https://example.com/', 120) + '";'), []);
});

test("comment markers inside literals are not comments", () => {
  const text = [
    'let a = "// not a comment";',
    'let b = "escaped \\" quote // still a string";',
    'let c = r#"a "quoted" // raw string"#;',
    'let d = br"// raw bytes";',
    "let e = ['\"', '/', '\\'', '\\u{2192}']; // 5",
    "fn f<'a>(x: &'a str) -> &'a str { x } // 6",
    'let g = "first line',
    '// still the string";',
    "let r#type = 1; // 9",
    "/* a block",
    "   comment */ let h = 1;",
  ].join("\n");
  assert.deepEqual([...commentLines(text)], [5, 6, 9, 10, 11]);
});
