use std::collections::BTreeSet;
use std::fmt::Display;

use regex_syntax::ast::{self, Ast};
use regex_syntax::hir::{Class, Hir, HirKind, Look};

use crate::tools::GrepArgs;

/// Lines of context `grep` shows at most before and after a matching line.
pub const MAX_CONTEXT: u64 = 100;

/// AI-48: a pattern as the host's `rg` and `grep -E` get it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The pattern as the model wrote it, which `rg` reads.
    pub rg: String,
    /// The same pattern for `grep -E`, where the host has no `rg`; `Err` says what in it only
    /// `rg` can run.
    pub extended: Result<String, &'static str>,
    /// Letter case is ignored: asked for, or by a `(?i)` that starts the pattern, which `grep -E`
    /// gets as `-i`.
    pub ignore_case: bool,
}

/// What only `rg` can run, for the message where the host has none.
pub(super) const MULTILINE: &str = "a search across lines (multiline)";
pub(super) const UNICODE_CLASS: &str = "Unicode classes such as \\p{L}";
pub(super) const FLAGS: &str = "inline flags other than a (?i) that starts the pattern";
pub(super) const TEXT_ANCHOR: &str = "\\A and \\z";
const HALF_BOUNDARY: &str = "\\b{start-half} and \\b{end-half}";
pub(super) const CLASS_OPERATION: &str = "class operations such as [a-z&&[^aeiou]]";
pub(super) const NEGATED_IN_CLASS: &str =
    "negated classes such as \\D or [:^alpha:] inside brackets";
pub(super) const ASCII_CLASS: &str = "[:ascii:]";
const SPECIAL_RANGE: &str = "a range that starts or ends with ] - ^ or [";
pub(super) const CONTROL: &str = "a carriage return or NUL";

/// AI-48: `pattern`, a regular expression as `rg` reads it (Rust's `regex` syntax), checked as
/// `rg` checks it, and translated for `grep -E` where the host has no `rg`, for the same lines:
/// `\d`, `\s` and `\w` and their negations as POSIX classes, `(?:…)` and named groups as plain
/// ones, lazy repetitions as greedy ones, which match the same lines, a leading `(?i)` as `-i`,
/// and escapes as `grep -E` reads them. An error is the message for the model.
pub fn pattern(given: &str, ignore_case: bool, multiline: bool) -> Result<Pattern, String> {
    if given.is_empty() {
        return Err("The pattern is empty, so nothing was searched.".to_owned());
    }
    if given.contains(['\n', '\r', '\0']) {
        return Err(
            "The pattern has a line break, so nothing was searched. Write a line break as \\n, \
             with multiline set to true."
                .to_owned(),
        );
    }
    let ast = ast::parse::Parser::new()
        .parse(given)
        .map_err(|e| invalid(e.kind(), e.span()))?;
    regex_syntax::hir::translate::TranslatorBuilder::new()
        .utf8(false)
        .build()
        .translate(given, &ast)
        .map_err(|e| invalid(e.kind(), e.span()))?;
    if !multiline && has_line_feed(&ast) {
        return Err(
            "The pattern matches a line break (\\n), and a search reads one line at a time unless \
             multiline is true, so nothing was searched. Set multiline to true to match across \
             lines."
                .to_owned(),
        );
    }
    let mut body = ast.clone();
    let leading_i = strip_case_flag(&mut body);
    let extended = if multiline {
        Err(MULTILINE)
    } else {
        let mut out = String::new();
        extended(&body, &mut out).map(|()| if out.is_empty() { "^".to_owned() } else { out })
    };
    Ok(Pattern {
        rg: given.to_owned(),
        extended,
        ignore_case: ignore_case || leading_i,
    })
}

fn invalid(kind: &impl Display, span: &ast::Span) -> String {
    format!(
        "The pattern is not a valid regular expression ({kind}, at character {}), so nothing was \
         searched. Escape ( ) [ ] {{ }} . * + ? | ^ $ and \\ with a backslash to match them as \
         they are.",
        span.start.column
    )
}

/// Takes out a `(?i)` that starts the pattern, also when it starts the first of its alternatives,
/// which it covers too: whether there was one. One inside a group covers only the group.
fn strip_case_flag(ast: &mut Ast) -> bool {
    match ast {
        Ast::Flags(set) if is_case_flag(&set.flags) => {
            *ast = Ast::empty(set.span);
            true
        }
        Ast::Concat(concat) => concat.asts.first_mut().is_some_and(strip_case_flag),
        Ast::Alternation(alternation) => alternation.asts.first_mut().is_some_and(strip_case_flag),
        _ => false,
    }
}

fn is_case_flag(flags: &ast::Flags) -> bool {
    matches!(
        &flags.items[..],
        [ast::FlagsItem {
            kind: ast::FlagsItemKind::Flag(ast::Flag::CaseInsensitive),
            ..
        }]
    )
}

/// Whether `ast` has a `\n` outside brackets, which `rg` refuses unless the search is multiline.
fn has_line_feed(ast: &Ast) -> bool {
    match ast {
        Ast::Literal(literal) => literal.c == '\n',
        Ast::Repetition(repetition) => has_line_feed(&repetition.ast),
        Ast::Group(group) => has_line_feed(&group.ast),
        Ast::Alternation(alternation) => alternation.asts.iter().any(has_line_feed),
        Ast::Concat(concat) => concat.asts.iter().any(has_line_feed),
        _ => false,
    }
}

/// Whether `rg -c` counts matches rather than matching lines for `args`: in a multiline search
/// whose pattern can take in a line break, or has `^`, `$`, `\A` or `\z`, for which rg reads files
/// whole, as its `non_matching_bytes` tells.
pub(super) fn counts_matches(args: &GrepArgs) -> bool {
    args.multiline.unwrap_or(false)
        && regex_syntax::ParserBuilder::new()
            .multi_line(true)
            .dot_matches_new_line(true)
            .utf8(false)
            .build()
            .parse(&args.pattern)
            .is_ok_and(|hir| takes_line_break(&hir))
}

fn takes_line_break(hir: &Hir) -> bool {
    match hir.kind() {
        HirKind::Empty => false,
        HirKind::Literal(literal) => literal.0.contains(&b'\n'),
        HirKind::Class(Class::Unicode(class)) => class
            .ranges()
            .iter()
            .any(|r| r.start() <= '\n' && '\n' <= r.end()),
        HirKind::Class(Class::Bytes(class)) => class
            .ranges()
            .iter()
            .any(|r| r.start() <= b'\n' && b'\n' <= r.end()),
        HirKind::Look(look) => matches!(
            look,
            Look::Start | Look::End | Look::StartLF | Look::EndLF | Look::StartCRLF | Look::EndCRLF
        ),
        HirKind::Repetition(repetition) => takes_line_break(&repetition.sub),
        HirKind::Capture(capture) => takes_line_break(&capture.sub),
        HirKind::Concat(all) | HirKind::Alternation(all) => all.iter().any(takes_line_break),
    }
}

/// Appends `ast` as `grep -E` reads it.
fn extended(ast: &Ast, out: &mut String) -> Result<(), &'static str> {
    match ast {
        Ast::Empty(_) => {}
        Ast::Flags(_) => return Err(FLAGS),
        Ast::Literal(literal) => match literal.c {
            '\r' | '\0' => return Err(CONTROL),
            c @ ('.' | '[' | '\\' | '(' | ')' | '*' | '+' | '?' | '{' | '|' | '^' | '$') => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        },
        Ast::Dot(_) => out.push('.'),
        Ast::Assertion(assertion) => out.push_str(match assertion.kind {
            ast::AssertionKind::StartLine => "^",
            ast::AssertionKind::EndLine => "$",
            ast::AssertionKind::WordBoundary => "\\b",
            ast::AssertionKind::NotWordBoundary => "\\B",
            ast::AssertionKind::WordBoundaryStart | ast::AssertionKind::WordBoundaryStartAngle => {
                "\\<"
            }
            ast::AssertionKind::WordBoundaryEnd | ast::AssertionKind::WordBoundaryEndAngle => "\\>",
            ast::AssertionKind::StartText | ast::AssertionKind::EndText => {
                return Err(TEXT_ANCHOR);
            }
            ast::AssertionKind::WordBoundaryStartHalf | ast::AssertionKind::WordBoundaryEndHalf => {
                return Err(HALF_BOUNDARY);
            }
        }),
        Ast::ClassUnicode(_) => return Err(UNICODE_CLASS),
        Ast::ClassPerl(class) => out.push_str(match (&class.kind, class.negated) {
            (ast::ClassPerlKind::Digit, false) => "[0-9]",
            (ast::ClassPerlKind::Digit, true) => "[^0-9]",
            (ast::ClassPerlKind::Space, false) => "[[:space:]]",
            (ast::ClassPerlKind::Space, true) => "[^[:space:]]",
            (ast::ClassPerlKind::Word, false) => "[[:alnum:]_]",
            (ast::ClassPerlKind::Word, true) => "[^[:alnum:]_]",
        }),
        Ast::ClassBracketed(class) => {
            let mut set = Bracket {
                negated: class.negated,
                ..Bracket::default()
            };
            match &class.kind {
                ast::ClassSet::Item(item) => set.add(item)?,
                ast::ClassSet::BinaryOp(_) => return Err(CLASS_OPERATION),
            }
            set.write(out)?;
        }
        Ast::Repetition(repetition) => {
            // A lazy repetition matches the same lines as a greedy one.
            extended(&repetition.ast, out)?;
            match &repetition.op.kind {
                ast::RepetitionKind::ZeroOrOne => out.push('?'),
                ast::RepetitionKind::ZeroOrMore => out.push('*'),
                ast::RepetitionKind::OneOrMore => out.push('+'),
                ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(n)) => {
                    out.push_str(&format!("{{{n}}}"));
                }
                ast::RepetitionKind::Range(ast::RepetitionRange::AtLeast(n)) => {
                    out.push_str(&format!("{{{n},}}"));
                }
                ast::RepetitionKind::Range(ast::RepetitionRange::Bounded(m, n)) => {
                    out.push_str(&format!("{{{m},{n}}}"));
                }
            }
        }
        Ast::Group(group) => {
            if group.flags().is_some_and(|flags| !flags.items.is_empty()) {
                return Err(FLAGS);
            }
            out.push('(');
            extended(&group.ast, out)?;
            out.push(')');
        }
        Ast::Alternation(alternation) => {
            for (i, ast) in alternation.asts.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                extended(ast, out)?;
            }
        }
        Ast::Concat(concat) => {
            for ast in &concat.asts {
                extended(ast, out)?;
            }
        }
    }
    Ok(())
}

/// The members of a bracket expression, gathered to be written as POSIX reads them.
#[derive(Default)]
struct Bracket {
    negated: bool,
    chars: BTreeSet<char>,
    ranges: Vec<(char, char)>,
    classes: Vec<&'static str>,
}

impl Bracket {
    fn add(&mut self, item: &ast::ClassSetItem) -> Result<(), &'static str> {
        match item {
            ast::ClassSetItem::Empty(_) => {}
            // A line break is never part of a line, as `rg` takes it out of a class.
            ast::ClassSetItem::Literal(literal) if literal.c == '\n' => {}
            ast::ClassSetItem::Literal(literal) => {
                if matches!(literal.c, '\r' | '\0') {
                    return Err(CONTROL);
                }
                self.chars.insert(literal.c);
            }
            ast::ClassSetItem::Range(range) => {
                let ends = [range.start.c, range.end.c];
                if ends.iter().any(|c| matches!(c, ']' | '-' | '^' | '[')) {
                    return Err(SPECIAL_RANGE);
                }
                if ends.iter().any(|c| matches!(c, '\n' | '\r' | '\0')) {
                    return Err(CONTROL);
                }
                self.ranges.push((range.start.c, range.end.c));
            }
            ast::ClassSetItem::Ascii(class) => {
                if class.negated {
                    return Err(NEGATED_IN_CLASS);
                }
                self.classes.push(match class.kind {
                    ast::ClassAsciiKind::Alnum => "alnum",
                    ast::ClassAsciiKind::Alpha => "alpha",
                    ast::ClassAsciiKind::Blank => "blank",
                    ast::ClassAsciiKind::Cntrl => "cntrl",
                    ast::ClassAsciiKind::Digit => "digit",
                    ast::ClassAsciiKind::Graph => "graph",
                    ast::ClassAsciiKind::Lower => "lower",
                    ast::ClassAsciiKind::Print => "print",
                    ast::ClassAsciiKind::Punct => "punct",
                    ast::ClassAsciiKind::Space => "space",
                    ast::ClassAsciiKind::Upper => "upper",
                    ast::ClassAsciiKind::Xdigit => "xdigit",
                    ast::ClassAsciiKind::Word => {
                        self.chars.insert('_');
                        "alnum"
                    }
                    ast::ClassAsciiKind::Ascii => return Err(ASCII_CLASS),
                });
            }
            ast::ClassSetItem::Unicode(_) => return Err(UNICODE_CLASS),
            ast::ClassSetItem::Perl(class) => {
                if class.negated {
                    return Err(NEGATED_IN_CLASS);
                }
                match class.kind {
                    ast::ClassPerlKind::Digit => self.ranges.push(('0', '9')),
                    ast::ClassPerlKind::Space => self.classes.push("space"),
                    ast::ClassPerlKind::Word => {
                        self.classes.push("alnum");
                        self.chars.insert('_');
                    }
                }
            }
            ast::ClassSetItem::Bracketed(inner) => match &inner.kind {
                _ if inner.negated => return Err(NEGATED_IN_CLASS),
                ast::ClassSet::Item(item) => self.add(item)?,
                ast::ClassSet::BinaryOp(_) => return Err(CLASS_OPERATION),
            },
            ast::ClassSetItem::Union(union) => {
                for item in &union.items {
                    self.add(item)?;
                }
            }
        }
        Ok(())
    }

    /// POSIX gives `]` a place first, `-` last and `^` anywhere but first; `\` stands for itself,
    /// and `[` for itself unless `:`, `.` or `=` follows.
    fn write(&self, out: &mut String) -> Result<(), &'static str> {
        let has = |c| self.chars.contains(&c);
        let mut body = String::new();
        if has(']') {
            body.push(']');
        }
        for &c in &self.chars {
            if !matches!(c, ']' | '-' | '^' | '[') {
                body.push(c);
            }
        }
        for &(low, high) in &self.ranges {
            body.extend([low, '-', high]);
        }
        for class in &self.classes {
            body.push_str(&format!("[:{class}:]"));
        }
        if has('[') {
            body.push('[');
        }
        let mut dash = has('-');
        if has('^') {
            if !body.is_empty() || self.negated {
                body.push('^');
            } else if dash {
                body.push_str("-^");
                dash = false;
            } else {
                // `[^]` would be a negation.
                out.push_str("\\^");
                return Ok(());
            }
        }
        if dash {
            body.push('-');
        }
        if body.is_empty() {
            // Only line breaks, which no line has: negated, any character of the line.
            if self.negated {
                out.push('.');
                return Ok(());
            }
            return Err(CONTROL);
        }
        out.push('[');
        if self.negated {
            out.push('^');
        }
        out.push_str(&body);
        out.push(']');
        Ok(())
    }
}

/// The lines of context before and after each matching line that `args` ask for: `-B` and `-A`,
/// or `context` for either, at most [`MAX_CONTEXT`] each.
#[must_use]
pub fn context(args: &GrepArgs) -> (u32, u32) {
    let lines = |side: Option<u64>| {
        u32::try_from(side.or(args.context).unwrap_or(0).min(MAX_CONTEXT)).unwrap_or_default()
    };
    (lines(args.before_context), lines(args.after_context))
}
