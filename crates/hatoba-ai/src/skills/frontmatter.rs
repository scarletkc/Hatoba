//! `SKILL.md`: YAML frontmatter between two `---` lines, then the Markdown body.

use serde_json::{Map, Value};

use super::{SkillIssue, SkillPackage};

/// Splits `SKILL.md` into its frontmatter fields (YAML converted to JSON, `name` and
/// `description` included) and its body, which is the text after the closing `---` line, as
/// written. A leading byte order mark is dropped and CRLF line endings are understood.
///
/// Every failure is an [`SkillIssue::InvalidFrontmatter`] that says what is wrong, with the
/// line of the file for YAML errors.
pub fn parse_skill_md(text: &str) -> Result<(Map<String, Value>, String), SkillIssue> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or_default();
    if first.trim_end() != "---" {
        return Err(invalid("SKILL.md must start with a `---` line"));
    }
    let yaml_start = first.len();
    let mut offset = yaml_start;
    for line in lines {
        if line.trim_end() == "---" {
            let fields = parse_yaml(&text[yaml_start..offset])?;
            return Ok((fields, text[offset + line.len()..].to_owned()));
        }
        offset += line.len();
    }
    Err(invalid("the frontmatter is not closed by a `---` line"))
}

fn invalid(message: &str) -> SkillIssue {
    SkillIssue::InvalidFrontmatter(message.to_owned())
}

fn parse_yaml(yaml: &str) -> Result<Map<String, Value>, SkillIssue> {
    if yaml.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_yaml_ng::from_str::<Value>(yaml) {
        Ok(Value::Object(fields)) => Ok(fields),
        Ok(Value::Null) => Ok(Map::new()),
        Ok(_) => Err(invalid("the frontmatter must be a mapping of fields")),
        Err(err) => Err(SkillIssue::InvalidFrontmatter(yaml_message(&err))),
    }
}

/// The YAML error with its position moved from "at line L column C" (inside the frontmatter)
/// to "(line N)" of the file, which has the opening `---` line above the YAML.
fn yaml_message(err: &serde_yaml_ng::Error) -> String {
    let message = err.to_string();
    if let Some(at) = err.location() {
        let suffix = format!(" at line {} column {}", at.line(), at.column());
        if let Some(text) = message.strip_suffix(&suffix) {
            return format!("{text} (line {})", at.line() + 1);
        }
    }
    message
}

/// Renders `SKILL.md`: a frontmatter with `name`, `description`, then every other field of
/// [`SkillPackage::frontmatter`] in its order, a closing `---` line, and the body as is.
///
/// What [`parse_skill_md`] reads back is the same fields and the same body.
#[must_use]
pub fn render_skill_md(pkg: &SkillPackage) -> String {
    let mut fields = serde_yaml_ng::Mapping::new();
    fields.insert("name".into(), pkg.name.as_str().into());
    fields.insert("description".into(), pkg.description.as_str().into());
    for (key, value) in &pkg.frontmatter {
        if key == "name" || key == "description" {
            continue;
        }
        if let Ok(value) = serde_yaml_ng::to_value(value) {
            fields.insert(key.as_str().into(), value);
        }
    }
    let yaml = serde_yaml_ng::to_string(&serde_yaml_ng::Value::Mapping(fields))
        .unwrap_or_else(|_| minimal_yaml(pkg));
    let sep = if yaml.ends_with('\n') { "" } else { "\n" };
    format!("---\n{yaml}{sep}---\n{}", pkg.body)
}

/// The two required fields as JSON strings, which are also YAML strings. Only reached if the
/// YAML serializer fails, which it does not for the values a package holds.
fn minimal_yaml(pkg: &SkillPackage) -> String {
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "name: {}\ndescription: {}\n",
        quote(&pkg.name),
        quote(&pkg.description)
    )
}
