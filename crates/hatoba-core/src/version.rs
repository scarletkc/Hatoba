//! Release versions of the app and the sync Worker.

use std::cmp::Ordering;
use std::fmt;

/// A release version, `MAJOR.MINOR.PATCH[-alpha|beta|rc.N]`, in the format and order of
/// `parseVersion` and `compareVersions` in `scripts/release/version.mjs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    /// `None` for a stable release.
    pre: Option<(Stage, u64)>,
}

/// Declared in release order, so the derived ordering is alpha < beta < rc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Alpha,
    Beta,
    Rc,
}

impl Version {
    /// Parses a release version; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let core = [
            number(parts.next()?)?,
            number(parts.next()?)?,
            number(parts.next()?)?,
        ];
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => None,
            Some(pre) => {
                let (stage, n) = pre.split_once('.')?;
                let stage = match stage {
                    "alpha" => Stage::Alpha,
                    "beta" => Stage::Beta,
                    "rc" => Stage::Rc,
                    _ => return None,
                };
                Some((stage, number(n)?))
            }
        };
        Some(Self { core, pre })
    }
}

/// A decimal number without leading zeros, as in the release format.
fn number(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}

impl Ord for Version {
    /// 1.2.3-alpha.9 < 1.2.3-alpha.10 < 1.2.3-beta.1 < 1.2.3-rc.1 < 1.2.3.
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre, other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(&b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        if let Some((stage, n)) = self.pre {
            let stage = match stage {
                Stage::Alpha => "alpha",
                Stage::Beta => "beta",
                Stage::Rc => "rc",
            };
            write!(f, "-{stage}.{n}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn parses_release_versions() {
        for text in [
            "0.1.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha.0",
            "1.0.0-beta.2",
            "2.0.0-rc.11",
        ] {
            assert_eq!(v(text).to_string(), text);
        }
    }

    #[test]
    fn rejects_other_versions() {
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "v1.2.3",
            "1.2.3-",
            "1.2.3-rc",
            "1.2.3-rc.01",
            "1.2.3-dev.1",
            "1.2.3-RC.1",
            "1.2.3-rc.1.2",
            "1.2.3+build",
            "1.2.-3",
            " 1.2.3",
            "1.2.x",
            "99999999999999999999.0.0",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?} should not parse");
        }
    }

    #[test]
    fn orders_like_the_release_scripts() {
        let ordered = [
            "0.9.9",
            "1.0.0-alpha.1",
            "1.0.0-alpha.9",
            "1.0.0-alpha.10",
            "1.0.0-beta.0",
            "1.0.0-beta.2",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "1.10.0",
            "2.0.0-rc.1",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("1.2.3").cmp(&v("1.2.3")), Ordering::Equal);
        assert_eq!(v("1.2.3-rc.2").cmp(&v("1.2.3-rc.2")), Ordering::Equal);
    }
}
