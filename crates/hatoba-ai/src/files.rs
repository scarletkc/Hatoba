//! The text side of the file tools (spec §13.4, AI-38…40): decoding a file, the numbered pages
//! `read_file` returns, the replacement `edit_file` makes, and the line breaks `write_file`
//! writes. Reading and writing on the host is the desktop shell's part.

use crate::tools::{EditFileArgs, MAX_RESULT_CHARS};

/// The largest file the tools read, and the largest file `edit_file` and `write_file` write.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Characters of a line `read_file` shows; the rest is cut.
pub const MAX_LINE_CHARS: usize = 2_000;
/// Characters of the changed lines an `edit_file` result shows.
const EDIT_SNIPPET_CHARS: usize = 4_000;
/// Lines shown around the changed lines in an `edit_file` result.
const EDIT_CONTEXT_LINES: usize = 2;

const BOM: char = '\u{feff}';

/// A text file as the tools see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFile {
    /// The text, without a byte order mark.
    pub text: String,
    /// The file starts with a UTF-8 byte order mark, which a write keeps.
    pub bom: bool,
    /// Most of its line breaks are CRLF.
    pub crlf: bool,
}

impl TextFile {
    /// Decodes a file's bytes. Refuses NUL bytes and invalid UTF-8, as attachments do (AI-35).
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.contains(&0) {
            return Err("it has NUL bytes, so it is not a text file");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "it is not valid UTF-8 text")?;
        let (text, bom) = match text.strip_prefix(BOM) {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        Ok(Self {
            text: text.to_owned(),
            bom,
            crlf: mostly_crlf(text),
        })
    }

    /// A file with `text` in this file's style: its byte order mark, and CRLF line breaks when
    /// most of its line breaks are CRLF (AI-40).
    #[must_use]
    pub fn with_text(&self, text: &str) -> Self {
        let text = if self.bom {
            text.strip_prefix(BOM).unwrap_or(text)
        } else {
            text
        };
        Self {
            text: if self.crlf {
                to_crlf(text)
            } else {
                text.to_owned()
            },
            bom: self.bom,
            crlf: self.crlf,
        }
    }

    /// The bytes to write.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = String::with_capacity(self.text.len() + 3);
        if self.bom {
            out.push(BOM);
        }
        out.push_str(&self.text);
        out.into_bytes()
    }

    /// The text with CRLF line breaks shown as `\n`, for the diff on the approval card: the
    /// write keeps the file's style, so the card shows what changes in the lines.
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.crlf {
            self.text.replace("\r\n", "\n")
        } else {
            self.text.clone()
        }
    }
}

fn mostly_crlf(text: &str) -> bool {
    let crlf = text.matches("\r\n").count();
    crlf > text.matches('\n').count() - crlf
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The file's lines without their line breaks; a final line break ends the last line instead of
/// starting an empty one.
fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// One numbered line as `cat -n` prints it, cut to [`MAX_LINE_CHARS`]. `open`: the line goes on
/// past what was read, so its length is not known. Returns whether it was cut.
fn numbered(out: &mut String, number: usize, line: &str, open: bool) -> bool {
    let total = line.chars().count();
    out.push_str(&format!("{number:>6}\t"));
    if total > MAX_LINE_CHARS || open {
        let end = line
            .char_indices()
            .nth(MAX_LINE_CHARS)
            .map_or(line.len(), |(i, _)| i);
        out.push_str(&line[..end]);
        if open {
            out.push_str(" [… cut]\n");
        } else {
            out.push_str(&format!(
                " [… cut: {} more characters]\n",
                total - MAX_LINE_CHARS
            ));
        }
        true
    } else {
        out.push_str(line);
        out.push('\n');
        false
    }
}

/// The most bytes of a file over [`MAX_FILE_BYTES`] that one `read_file` call reads from the host,
/// from the line it starts at: more than a page shows.
pub const MAX_PAGE_BYTES: u64 = 256 * 1024;

/// AI-38: the page of `file` that `read_file` returns, from line `offset` (1-based, 1 by default),
/// at most `limit` lines, and at most [`MAX_RESULT_CHARS`] characters in all. `path` is the path as
/// the model gave it. An error is the message for the model.
pub fn read_page(
    path: &str,
    file: &TextFile,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<String, String> {
    let all = lines(&file.text);
    let total = all.len();
    if total == 0 {
        return Ok(format!("{path} is empty.\n"));
    }
    let first = usize::try_from(offset.unwrap_or(1).max(1)).unwrap_or(usize::MAX);
    if first > total {
        return Err(past_end(path, total, first));
    }
    Ok(page(
        path,
        &all[first - 1..],
        first,
        total,
        limit,
        file.crlf,
        false,
    ))
}

/// AI-38: a page of a file over [`MAX_FILE_BYTES`], which `read_file` reads a part at a time:
/// `bytes` are the file's bytes from the start of line `offset` on, `cut` when more of the file
/// followed them, and `total` is its line count. Shown as [`read_page`] shows a page, with a note
/// that the file is too large for `edit_file` and `write_file`.
pub fn read_large_page(
    path: &str,
    offset: u64,
    total: u64,
    bytes: &[u8],
    cut: bool,
    limit: Option<u64>,
) -> Result<String, String> {
    let total = usize::try_from(total).unwrap_or(usize::MAX);
    if total == 0 {
        return Ok(format!("{path} is empty.\n"));
    }
    let first = usize::try_from(offset.max(1)).unwrap_or(usize::MAX);
    let not_text = |why: &str| {
        format!("{path} cannot be read as text: {why}. The file tools read UTF-8 text files only.")
    };
    if bytes.contains(&0) {
        return Err(not_text("it has NUL bytes, so it is not a text file"));
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        // The part read can end inside a character.
        Err(e) if cut && e.error_len().is_none() => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return Err(not_text("it is not valid UTF-8 text")),
    };
    let text = if first == 1 {
        text.strip_prefix(BOM).unwrap_or(text)
    } else {
        text
    };
    let mut shown = lines(text);
    // The last line goes on past what was read: it shows only when it is the only one, and the
    // next page starts with it otherwise.
    let mut open = cut && !text.ends_with('\n');
    if open && shown.len() > 1 {
        shown.pop();
        open = false;
    }
    if first > total || shown.is_empty() {
        return Err(past_end(path, total, first));
    }
    let mut out = page(path, &shown, first, total, limit, mostly_crlf(text), open);
    out.push_str("[The file is larger than 1 MB, so edit_file and write_file do not change it.]\n");
    Ok(out)
}

fn past_end(path: &str, total: usize, first: usize) -> String {
    format!("{path} has {total} lines, so offset {first} is past its end.")
}

/// A page's text: the header, the numbered lines of `shown` (the lines from `first` on that are at
/// hand) that fit `limit` and the size of a result, and how to read on. `open`: the last of
/// `shown` goes on past what was read.
fn page(
    path: &str,
    shown: &[&str],
    first: usize,
    total: usize,
    limit: Option<u64>,
    crlf: bool,
    open: bool,
) -> String {
    let limit = limit
        .map(|l| usize::try_from(l.max(1)).unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX);
    let crlf = if crlf { ", CRLF line breaks" } else { "" };
    // The header and the closing notes take at most this much of the page.
    let budget = MAX_RESULT_CHARS - 400 - path.chars().count();
    let mut body = String::new();
    let mut used = 0;
    let mut last = first - 1;
    let mut cut = false;
    for (i, line) in shown.iter().enumerate().take(limit) {
        let number = first + i;
        let mut next = String::new();
        let was_cut = numbered(&mut next, number, line, open && i + 1 == shown.len());
        let size = next.chars().count();
        if used + size > budget && last >= first {
            break;
        }
        used += size;
        body.push_str(&next);
        cut |= was_cut;
        last = number;
    }
    let mut out = if first == 1 && last == total {
        format!("{path}: {}{crlf}\n", line_count(total))
    } else {
        format!("{path}: lines {first}–{last} of {total}{crlf}\n")
    };
    out.push_str(&body);
    if last < total {
        out.push_str(&format!(
            "[Lines {}–{total} are not shown. To read on, call read_file with offset {}.]\n",
            last + 1,
            last + 1
        ));
    }
    if cut {
        out.push_str(&format!(
            "[Lines longer than {MAX_LINE_CHARS} characters are cut. run_command can print one \
             whole, for example with sed -n.]\n"
        ));
    }
    out
}

/// The result of an `edit_file` replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// The file after the replacement.
    pub file: TextFile,
    /// How many occurrences were replaced.
    pub count: usize,
    /// 0-based line of the first replacement in the new text.
    first_line: usize,
    /// Lines the first replacement spans in the new text.
    span: usize,
}

/// AI-39: replaces `old_string` with `new_string` in `file`. `old_string` matches CRLF and LF line
/// breaks alike, as `read_file` shows neither; in a file whose line breaks are mostly CRLF,
/// `new_string`'s are written as CRLF. An error is the message for the model.
pub fn apply_edit(path: &str, file: &TextFile, args: &EditFileArgs) -> Result<Edit, String> {
    if args.old_string.is_empty() {
        return Err(
            "old_string is empty. To create a file or replace all of its content, use write_file."
                .to_owned(),
        );
    }
    let old = args.old_string.replace("\r\n", "\n");
    if old == args.new_string.replace("\r\n", "\n") {
        return Err(
            "old_string and new_string are the same, so there is nothing to change.".to_owned(),
        );
    }
    let new = if file.crlf {
        to_crlf(&args.new_string)
    } else {
        args.new_string.clone()
    };
    // The file with each CRLF as LF, where the CRLFs were in it, and where a byte of it is in the
    // file: a CRLF's LF is at its CR, so a match that takes the line break takes both bytes.
    let lf = file.text.replace("\r\n", "\n");
    let crlfs: Vec<usize> = file
        .text
        .match_indices("\r\n")
        .enumerate()
        .map(|(k, (at, _))| at - k)
        .collect();
    let in_file = |at: usize| at + crlfs.partition_point(|&c| c < at);
    let found: Vec<usize> = lf.match_indices(old.as_str()).map(|(at, _)| at).collect();
    let count = found.len();
    if count == 0 {
        return Err(format!(
            "old_string was not found in {path}, so nothing was changed. It must match the file \
             exactly, including spaces, tabs and line breaks: read the file with read_file and \
             copy the text without the line numbers."
        ));
    }
    let replace_all = args.replace_all.unwrap_or(false);
    if count > 1 && !replace_all {
        return Err(format!(
            "old_string appears {count} times in {path}, so nothing was changed. Include more of \
             the surrounding lines to make it unique, or set replace_all to replace every \
             occurrence."
        ));
    }
    let replaced = &found[..if replace_all { count } else { 1 }];
    // The size comes first: replacing each of a million short matches with a long string would
    // otherwise build gigabytes before the check.
    let matched: usize = replaced
        .iter()
        .map(|&at| in_file(at + old.len()) - in_file(at))
        .sum();
    let size = (file.text.len() - matched) as u64
        + (replaced.len() as u64).saturating_mul(new.len() as u64)
        + if file.bom { 3 } else { 0 };
    if size > MAX_FILE_BYTES {
        return Err(too_large(path, size));
    }
    let mut text = String::with_capacity(file.text.len());
    let mut copied = 0;
    for &at in replaced {
        text.push_str(&file.text[copied..in_file(at)]);
        text.push_str(&new);
        copied = in_file(at + old.len());
    }
    text.push_str(&file.text[copied..]);
    let edited = TextFile {
        text,
        bom: file.bom,
        crlf: file.crlf,
    };
    check_result(path, &edited)?;
    Ok(Edit {
        first_line: lf[..found[0]].matches('\n').count(),
        span: new.matches('\n').count() + 1,
        file: edited,
        count,
    })
}

/// Refuses a result over [`MAX_FILE_BYTES`] or with a NUL byte: the tools write text files only,
/// as they read only those.
pub fn check_result(path: &str, file: &TextFile) -> Result<(), String> {
    if file.text.contains('\0') {
        return Err(format!(
            "{path} would have a NUL byte, and the file tools write text files only, so nothing \
             was written."
        ));
    }
    let size = file.text.len() as u64 + if file.bom { 3 } else { 0 };
    if size > MAX_FILE_BYTES {
        Err(too_large(path, size))
    } else {
        Ok(())
    }
}

fn too_large(path: &str, size: u64) -> String {
    format!(
        "{path} would be {size} bytes, and the file tools write files up to 1 MB, so nothing was \
         written."
    )
}

/// The `edit_file` result: how many occurrences were replaced, and the changed lines with a
/// little context, numbered as `read_file` numbers them.
#[must_use]
pub fn edit_result(path: &str, edit: &Edit) -> String {
    let all = lines(&edit.file.text);
    let replaced = if edit.count == 1 {
        "1 occurrence".to_owned()
    } else {
        format!("{} occurrences", edit.count)
    };
    let mut out = format!("Edited {path}: replaced {replaced} of old_string.\n");
    if all.is_empty() {
        out.push_str("The file is now empty.\n");
        return out;
    }
    let from = edit.first_line.saturating_sub(EDIT_CONTEXT_LINES);
    let to = (edit.first_line + edit.span + EDIT_CONTEXT_LINES).min(all.len());
    let mut body = String::new();
    let mut last = from;
    for (i, line) in all.iter().enumerate().take(to).skip(from) {
        let mut next = String::new();
        numbered(&mut next, i + 1, line, false);
        if body.chars().count() + next.chars().count() > EDIT_SNIPPET_CHARS && last > from {
            break;
        }
        body.push_str(&next);
        last = i + 1;
    }
    let first = if edit.count > 1 {
        "The first change"
    } else {
        "The change"
    };
    out.push_str(&format!(
        "{first}, lines {}–{last} of {}:\n{body}",
        from + 1,
        all.len()
    ));
    out
}

fn line_count(n: usize) -> String {
    if n == 1 {
        "1 line".to_owned()
    } else {
        format!("{n} lines")
    }
}

/// The `write_file` result.
#[must_use]
pub fn write_result(path: &str, file: &TextFile, created: bool) -> String {
    let lines = line_count(lines(&file.text).len());
    let crlf = if file.crlf {
        ", with the file's CRLF line breaks"
    } else {
        ""
    };
    if created {
        format!("Created {path} ({lines}).\n")
    } else {
        format!("Replaced the content of {path} ({lines}{crlf}).\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> TextFile {
        TextFile::decode(text.as_bytes()).unwrap()
    }

    fn edit(old: &str, new: &str, all: bool) -> EditFileArgs {
        EditFileArgs {
            path: "/etc/app.conf".into(),
            old_string: old.into(),
            new_string: new.into(),
            replace_all: Some(all),
        }
    }

    #[test]
    fn decoding_refuses_binary_and_keeps_the_bom() {
        assert!(TextFile::decode(b"a\0b").is_err());
        assert!(TextFile::decode(b"\xff\xfe").is_err());
        let f = TextFile::decode("\u{feff}a\r\nb\r\nc\n".as_bytes()).unwrap();
        assert_eq!(
            (f.text.as_str(), f.bom, f.crlf),
            ("a\r\nb\r\nc\n", true, true)
        );
        assert_eq!(f.encode(), "\u{feff}a\r\nb\r\nc\n".as_bytes());
        assert_eq!(f.display_text(), "a\nb\nc\n");
        // Ties and LF files stay LF.
        assert!(!file("a\r\nb\n").crlf);
        assert!(!file("").crlf);
        // A write in a CRLF file's style.
        assert_eq!(f.with_text("x\ny\r\n").text, "x\r\ny\r\n");
        assert_eq!(file("a\n").with_text("x\r\ny").text, "x\r\ny");
    }

    #[test]
    fn pages_are_numbered_and_say_how_to_read_on() {
        let f = file("one\r\ntwo\r\nthree\r\n");
        assert_eq!(
            read_page("a.txt", &f, None, None).unwrap(),
            "a.txt: 3 lines, CRLF line breaks\n     1\tone\n     2\ttwo\n     3\tthree\n"
        );
        assert_eq!(
            read_page("a.txt", &f, Some(2), Some(1)).unwrap(),
            "a.txt: lines 2–2 of 3, CRLF line breaks\n     2\ttwo\n[Lines 3–3 are not shown. To \
             read on, call read_file with offset 3.]\n"
        );
        assert_eq!(
            read_page("a.txt", &file("x"), Some(0), Some(0)).unwrap(),
            "a.txt: 1 line\n     1\tx\n"
        );
        assert_eq!(
            read_page("e", &file(""), None, None).unwrap(),
            "e is empty.\n"
        );
        assert_eq!(
            read_page("a.txt", &f, Some(4), None).unwrap_err(),
            "a.txt has 3 lines, so offset 4 is past its end."
        );

        // A long file stops at the size of a result, on a whole line.
        let text: String = (0..5_000).map(|i| format!("line {i}\n")).collect();
        let page = read_page("big", &file(&text), None, None).unwrap();
        assert!(page.chars().count() <= MAX_RESULT_CHARS, "{}", page.len());
        let last = page.lines().rev().nth(1).unwrap();
        let shown: usize = last.split('\t').next().unwrap().trim().parse().unwrap();
        assert!(page.starts_with(&format!("big: lines 1–{shown} of 5000\n")));
        assert!(page.ends_with(&format!(
            "To read on, call read_file with offset {}.]\n",
            shown + 1
        )));

        // A long line is cut, and a single line always shows.
        let long = format!("{}\n", "日".repeat(MAX_LINE_CHARS + 5));
        let page = read_page("min.js", &file(&long), None, None).unwrap();
        assert!(page.contains(" [… cut: 5 more characters]\n"));
        assert!(page.contains("are cut. run_command"));
        assert_eq!(page.matches('日').count(), MAX_LINE_CHARS);
    }

    #[test]
    fn large_files_are_read_a_part_at_a_time() {
        const LARGE: &str = "[The file is larger than 1 MB, so edit_file and write_file do not \
                             change it.]\n";
        // Lines 41 on of a file of 90 lines, the part ending inside line 43 and inside a character.
        let part = "forty-one\nforty-two\nforty-日".as_bytes();
        let part = &part[..part.len() - 1];
        assert_eq!(
            read_large_page("big.log", 41, 90, part, true, None).unwrap(),
            format!(
                "big.log: lines 41–42 of 90\n    41\tforty-one\n    42\tforty-two\n[Lines 43–90 \
                 are not shown. To read on, call read_file with offset 43.]\n{LARGE}"
            )
        );
        // The end of the file, its last line without a line break.
        assert_eq!(
            read_large_page("big.log", 90, 90, b"ninety", false, Some(5)).unwrap(),
            format!("big.log: lines 90–90 of 90\n    90\tninety\n{LARGE}")
        );
        // A line longer than what one call reads shows cut, and the next call goes past it.
        let long = format!("\u{feff}{}", "a".repeat(5_000));
        let page = read_large_page("min.js", 1, 2, long.as_bytes(), true, None).unwrap();
        assert!(
            page.starts_with("min.js: lines 1–1 of 2\n     1\ta"),
            "{page}"
        );
        assert!(page.contains(
            " [… cut]\n[Lines 2–2 are not shown. To read on, call read_file \
                               with offset 2.]\n"
        ));

        assert_eq!(
            read_large_page("e", 1, 0, b"", false, None).unwrap(),
            "e is empty.\n"
        );
        assert_eq!(
            read_large_page("big.log", 91, 90, b"", false, None).unwrap_err(),
            "big.log has 90 lines, so offset 91 is past its end."
        );
        assert!(read_large_page("blob", 1, 3, b"a\0b\n", true, None).is_err());
        assert!(read_large_page("blob", 1, 3, b"\xff\xfeab\n", true, None).is_err());
    }

    #[test]
    fn edits_match_exactly_once_unless_replace_all() {
        let f = file("server {\n    listen 80;\n    listen [::]:80;\n}\n");
        let e = apply_edit(
            "/etc/app.conf",
            &f,
            &edit("listen 80;", "listen 8080;", false),
        )
        .unwrap();
        assert_eq!(e.count, 1);
        assert_eq!(
            e.file.text,
            "server {\n    listen 8080;\n    listen [::]:80;\n}\n"
        );
        assert_eq!(
            edit_result("/etc/app.conf", &e),
            "Edited /etc/app.conf: replaced 1 occurrence of old_string.\nThe change, lines 1–4 of \
             4:\n     1\tserver {\n     2\t    listen 8080;\n     3\t    listen [::]:80;\n     \
             4\t}\n"
        );

        let not_found = apply_edit("/etc/app.conf", &f, &edit("listen 81;", "x", false));
        assert!(
            not_found
                .unwrap_err()
                .starts_with("old_string was not found in /etc/app.conf")
        );
        let twice = apply_edit("/etc/app.conf", &f, &edit("80;", "8080;", false));
        assert!(twice.unwrap_err().starts_with("old_string appears 2 times"));
        let all = apply_edit("/etc/app.conf", &f, &edit("80;", "8080;", true)).unwrap();
        assert_eq!(all.count, 2);
        assert!(edit_result("p", &all).contains("replaced 2 occurrences"));
        assert!(edit_result("p", &all).contains("The first change"));
        assert!(
            apply_edit("p", &f, &edit("", "x", false))
                .unwrap_err()
                .contains("write_file")
        );
        assert!(
            apply_edit("p", &f, &edit("80", "80", false))
                .unwrap_err()
                .contains("the same")
        );
    }

    #[test]
    fn edits_keep_crlf_line_breaks_and_the_bom() {
        let f = TextFile::decode("\u{feff}[a]\r\nx=1\r\ny=2\r\n".as_bytes()).unwrap();
        // The model writes \n, as read_file shows the lines.
        let e = apply_edit("w.ini", &f, &edit("x=1\ny=2\n", "x=1\ny=3\nz=4\n", false)).unwrap();
        assert_eq!(
            e.file.encode(),
            "\u{feff}[a]\r\nx=1\r\ny=3\r\nz=4\r\n".as_bytes()
        );
        // Line breaks match whichever kind the file has at that line, as read_file shows neither.
        let mixed = file("a\nb\r\nc\n");
        assert!(!mixed.crlf);
        let e = apply_edit("p", &mixed, &edit("a\nb\nc", "a\nB\nc", false)).unwrap();
        assert_eq!(e.file.text, "a\nB\nc\n");
        let e = apply_edit("p", &mixed, &edit("b", "B", false)).unwrap();
        assert_eq!(e.file.text, "a\nB\r\nc\n");
        assert_eq!(
            apply_edit("p", &file("a\nb\n"), &edit("a\r\nb", "c", false))
                .unwrap()
                .file
                .text,
            "c\n"
        );
        let crlf_with_lf = file("[a]\r\nx=1\ny=2\r\nx=1\r\ny=2\r\n");
        assert!(crlf_with_lf.crlf);
        let e = apply_edit("p", &crlf_with_lf, &edit("x=1\ny=2", "x=0", true)).unwrap();
        assert_eq!(e.count, 2);
        assert_eq!(e.file.text, "[a]\r\nx=0\r\nx=0\r\n");
        // A match that starts with a line break takes the whole CRLF.
        let e = apply_edit("p", &crlf_with_lf, &edit("]\nx=1\n", "]\nx=9\n", false)).unwrap();
        assert_eq!(e.file.text, "[a]\r\nx=9\r\ny=2\r\nx=1\r\ny=2\r\n");
        assert_eq!(e.first_line, 0);
    }

    #[test]
    fn results_over_the_limit_are_refused() {
        // Refused before the result is built: a megabyte of matches, each made 4,096 bytes.
        let many = file(&"a".repeat(MAX_FILE_BYTES as usize));
        let error = apply_edit("p", &many, &edit("a", &"b".repeat(4_096), true)).unwrap_err();
        assert!(error.contains("4294967296 bytes"), "{error}");
        // The exact size, BOM and CRLF included.
        let crlf = TextFile::decode("\u{feff}x\r\ny\r\n".as_bytes()).unwrap();
        let fits = "z".repeat(MAX_FILE_BYTES as usize - 3 - 4 - 1);
        assert!(apply_edit("p", &crlf, &edit("x\n", &format!("{fits}\n"), false)).is_ok());
        let over = format!("{fits}z\n");
        assert!(apply_edit("p", &crlf, &edit("x\n", &over, false)).is_err());

        let f = file("x\n");
        let big = "y".repeat(MAX_FILE_BYTES as usize);
        assert!(
            apply_edit("p", &f, &edit("x", &big, false))
                .unwrap_err()
                .contains("1 MB")
        );
        assert!(check_result("p", &file(&"y".repeat(MAX_FILE_BYTES as usize))).is_ok());
        // What the tools write, they can read back.
        assert!(
            apply_edit("p", &f, &edit("x", "x\0", false))
                .unwrap_err()
                .contains("NUL")
        );
        assert_eq!(
            write_result("/tmp/n", &file("a\nb"), true),
            "Created /tmp/n (2 lines).\n"
        );
        assert_eq!(
            write_result("/tmp/n", &file("a\r\n"), false),
            "Replaced the content of /tmp/n (1 line, with the file's CRLF line breaks).\n"
        );
    }
}
