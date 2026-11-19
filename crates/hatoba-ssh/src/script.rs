//! Framing and output parsing for the single-line `sh -c` scripts run on remote hosts.

/// Line printed before the resolved path so login-shell startup output is skipped.
const MARKER: &[u8] = b"hatoba-file\n";

/// `sh -c '<script>' sh [arg]`: a command line every common login shell reads the same way.
pub(crate) fn command(script: &str, arg: Option<&str>) -> String {
    debug_assert!(!script.contains(['\'', '\\', '\n']));
    match arg {
        Some(arg) => format!("sh -c '{script}' sh {arg}"),
        None => format!("sh -c '{script}' sh"),
    }
}

pub(crate) fn is_single_line(value: &str) -> bool {
    !value.contains(['\n', '\r', '\0'])
}

pub(crate) fn check_line(value: &str, allow_empty: bool) -> Option<&str> {
    ((allow_empty || !value.is_empty()) && is_single_line(value)).then_some(value)
}

pub(crate) fn last_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// What follows the marker line in a script's stdout: the resolved path and the rest. `None`
/// without a marker and a path line after it. Lines a startup file printed come before the
/// marker.
pub(crate) fn after_marker(stdout: &[u8]) -> Option<(String, &[u8])> {
    let start = if stdout.starts_with(MARKER) {
        0
    } else {
        stdout
            .windows(MARKER.len() + 1)
            .position(|w| w[0] == b'\n' && &w[1..] == MARKER)?
            + 1
    };
    let rest = &stdout[start + MARKER.len()..];
    let end = rest.iter().position(|&b| b == b'\n')?;
    let path = String::from_utf8_lossy(&rest[..end]).into_owned();
    Some((path, &rest[end + 1..]))
}
