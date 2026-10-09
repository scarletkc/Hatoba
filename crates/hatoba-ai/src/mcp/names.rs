//! Tool names the model sees (AI-30): `mcp__<server>__<tool>`.

use std::collections::HashSet;

/// Longest tool name both protocols accept (`^[A-Za-z0-9_-]{1,64}$`).
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// AI-30 tool names: `mcp__<server>__<tool>` with every character other than ASCII letters,
/// digits, `_` and `-` replaced by `_`, cut to 64 characters. Names that still collide get a
/// numeric suffix (`_2`, `_3`, …, cutting more so the name stays within 64), in input order:
/// the first holder of a name keeps it, and a suffixed name never takes a name another tool has
/// without a suffix.
///
/// Deterministic for the same input. Returns the names in input order, one `Vec` per server,
/// so the caller can map a name the model uses back to its server and tool.
#[must_use]
pub fn tool_names(servers: &[(&str, Vec<&str>)]) -> Vec<Vec<String>> {
    let bases: Vec<Vec<String>> = servers
        .iter()
        .map(|(server, tools)| tools.iter().map(|tool| base_name(server, tool)).collect())
        .collect();
    let unsuffixed: HashSet<&str> = bases.iter().flatten().map(String::as_str).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut names = Vec::with_capacity(bases.len());
    for server in &bases {
        let mut server_names = Vec::with_capacity(server.len());
        for base in server {
            let name = if taken.contains(base) {
                (2..)
                    .map(|n| with_suffix(base, n))
                    .find(|candidate| {
                        !unsuffixed.contains(candidate.as_str()) && !taken.contains(candidate)
                    })
                    .expect("the suffixes are unbounded")
            } else {
                base.clone()
            };
            taken.insert(name.clone());
            server_names.push(name);
        }
        names.push(server_names);
    }
    names
}

/// The cleaned, cut name before collisions are resolved.
fn base_name(server: &str, tool: &str) -> String {
    let mut name = format!("mcp__{}__{}", clean(server.trim()), clean(tool.trim()));
    // ASCII only after `clean`, so cutting bytes cuts characters.
    name.truncate(MAX_TOOL_NAME_LEN);
    name
}

fn clean(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn with_suffix(base: &str, n: u32) -> String {
    let suffix = format!("_{n}");
    let keep = base.len().min(MAX_TOOL_NAME_LEN - suffix.len());
    format!("{}{suffix}", &base[..keep])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(name: &str) -> bool {
        (1..=MAX_TOOL_NAME_LEN).contains(&name.len())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    #[test]
    fn names_are_cleaned() {
        let names = tool_names(&[
            ("fs", vec!["read_file", "write-file"]),
            (" My Server ", vec!["get.weather", "日本語", ""]),
        ]);
        assert_eq!(
            names,
            vec![
                vec!["mcp__fs__read_file", "mcp__fs__write-file"],
                vec![
                    "mcp__My_Server__get_weather",
                    "mcp__My_Server_____",
                    "mcp__My_Server__"
                ],
            ]
        );
        assert!(names.iter().flatten().all(|n| valid(n)));
    }

    #[test]
    fn long_names_are_cut_to_64() {
        let long_tool = "t".repeat(100);
        let names = tool_names(&[("server", vec![long_tool.as_str()])]);
        assert_eq!(names[0][0].len(), MAX_TOOL_NAME_LEN);
        assert!(names[0][0].starts_with("mcp__server__ttt"));
    }

    #[test]
    fn collisions_get_suffixes_within_64() {
        let a = format!("{}a", "x".repeat(80));
        let b = format!("{}b", "x".repeat(80));
        let names = tool_names(&[
            ("s", vec!["get.x", "get_x", "get x"]),
            ("s", vec!["get_x_2"]),
            ("long", vec![a.as_str(), b.as_str()]),
        ]);
        // `get_x_2` keeps its own name; the second `get_x` skips it.
        assert_eq!(
            names[0],
            vec!["mcp__s__get_x", "mcp__s__get_x_3", "mcp__s__get_x_4"]
        );
        assert_eq!(names[1], vec!["mcp__s__get_x_2"]);
        assert_eq!(names[2][0].len(), MAX_TOOL_NAME_LEN);
        assert_eq!(names[2][1].len(), MAX_TOOL_NAME_LEN);
        assert!(names[2][1].ends_with("_2"));
        assert_eq!(&names[2][0][..62], &names[2][1][..62]);

        let all: Vec<&String> = names.iter().flatten().collect();
        let unique: HashSet<&String> = all.iter().copied().collect();
        assert_eq!(all.len(), unique.len());
        assert!(all.iter().all(|n| valid(n)));
    }

    #[test]
    fn names_are_deterministic() {
        let input = [("a b", vec!["x", "x", "y"]), ("a_b", vec!["x"])];
        assert_eq!(tool_names(&input), tool_names(&input));
        assert_eq!(
            tool_names(&input),
            vec![
                vec!["mcp__a_b__x", "mcp__a_b__x_2", "mcp__a_b__y"],
                vec!["mcp__a_b__x_3"]
            ]
        );
        assert!(tool_names(&[]).is_empty());
    }
}
