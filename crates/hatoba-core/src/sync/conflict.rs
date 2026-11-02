//! Conflict resolution rules (spec §6.4) as pure functions.
//!
//! A conflict exists when an item has unpushed local changes *and* the server holds a newer
//! revision. Both versions are decrypted by the caller and handed to [`decide`]:
//!
//! 1. Default: last writer wins by the plaintext `updated_at`. On a tie the **remote** wins, so
//!    two devices facing the same tie converge on the same winner.
//! 2. SSH keys (`type = "key"`) are never silently dropped: when both sides are live and
//!    differ, the loser is preserved as a new item named with a "conflict copy" suffix.
//! 3. Delete vs modify: the modified side wins (the item is resurrected).
//! 4. Delete vs delete: the tombstone stands.
//! 5. Every automatic resolution that discards content is written to the local conflict log.

use serde::Serialize;

use crate::model::Item;

/// Suffix appended to the name of a preserved conflict copy. The desktop app passes the
/// localized text instead.
pub const DEFAULT_CONFLICT_SUFFIX: &str = " (conflict copy)";

/// One side of a conflict, already decrypted.
#[derive(Clone, Copy, Debug)]
pub struct Side<'a> {
    /// Whether this side is a tombstone.
    pub deleted: bool,
    /// The decrypted item; `None` for a tombstone.
    pub item: Option<&'a Item>,
    /// Plaintext `updated_at` for live items, row `updated_at` for tombstones.
    pub updated_at: i64,
}

impl<'a> Side<'a> {
    /// A live item.
    #[must_use]
    pub fn live(item: &'a Item) -> Self {
        Self {
            deleted: false,
            item: Some(item),
            updated_at: item.updated_at(),
        }
    }

    /// A tombstone with the time it was written.
    #[must_use]
    pub fn tombstone(updated_at: i64) -> Self {
        Self {
            deleted: true,
            item: None,
            updated_at,
        }
    }
}

/// What to do with a conflicting pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing was lost: identical content, or both sides deleted. Adopt the remote state.
    Converged,
    /// Keep the local version (it will be pushed on top of the server revision).
    KeepLocal {
        /// Also save the discarded remote version as a new conflict-copy item.
        copy_remote: bool,
    },
    /// Keep the remote version (the local version is discarded).
    KeepRemote {
        /// Also save the discarded local version as a new conflict-copy item.
        copy_local: bool,
    },
}

impl Decision {
    /// The log classification, or `None` if nothing needs logging.
    #[must_use]
    pub fn resolution(self) -> Option<Resolution> {
        match self {
            Self::Converged => None,
            Self::KeepLocal { copy_remote: false } => Some(Resolution::LocalWins),
            Self::KeepLocal { copy_remote: true } => Some(Resolution::LocalWinsRemoteCopied),
            Self::KeepRemote { copy_local: false } => Some(Resolution::RemoteWins),
            Self::KeepRemote { copy_local: true } => Some(Resolution::RemoteWinsLocalCopied),
        }
    }
}

/// How a logged conflict was resolved; stored as text in `conflict_log.resolution`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The local version won; the remote version was discarded.
    LocalWins,
    /// The remote version won; the local version was discarded.
    RemoteWins,
    /// The local version won; the remote version was kept as a conflict copy.
    LocalWinsRemoteCopied,
    /// The remote version won; the local version was kept as a conflict copy.
    RemoteWinsLocalCopied,
}

impl Resolution {
    /// The persisted text form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalWins => "local_wins",
            Self::RemoteWins => "remote_wins",
            Self::LocalWinsRemoteCopied => "local_wins_remote_copied",
            Self::RemoteWinsLocalCopied => "remote_wins_local_copied",
        }
    }

    /// Parses the persisted text form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "local_wins" => Self::LocalWins,
            "remote_wins" => Self::RemoteWins,
            "local_wins_remote_copied" => Self::LocalWinsRemoteCopied,
            "remote_wins_local_copied" => Self::RemoteWinsLocalCopied,
            _ => return None,
        })
    }

    /// Whether the local version was the winner.
    #[must_use]
    pub fn local_won(self) -> bool {
        matches!(self, Self::LocalWins | Self::LocalWinsRemoteCopied)
    }
}

/// Applies the §6.4 rules to a local/remote pair.
#[must_use]
pub fn decide(local: &Side<'_>, remote: &Side<'_>) -> Decision {
    match (local.item, remote.item) {
        // Delete vs delete: the tombstone stands.
        (None, None) => Decision::Converged,
        // Delete vs modify: the modified side wins, so the item comes back.
        (None, Some(_)) => Decision::KeepRemote { copy_local: false },
        (Some(_), None) => Decision::KeepLocal { copy_remote: false },
        (Some(l), Some(r)) => {
            if l == r {
                return Decision::Converged;
            }
            let preserve_loser = l.is_key() || r.is_key();
            // Last writer wins; a tie goes to the remote so every device picks the same winner.
            if local.updated_at > remote.updated_at {
                Decision::KeepLocal {
                    copy_remote: preserve_loser,
                }
            } else {
                Decision::KeepRemote {
                    copy_local: preserve_loser,
                }
            }
        }
    }
}

/// Builds the conflict copy of a discarded version: the same content under a suffixed name,
/// stamped `now` so it counts as a fresh edit and propagates.
#[must_use]
pub fn conflict_copy(loser: &Item, suffix: &str, now: i64) -> Item {
    let mut copy = loser.with_name_suffix(suffix);
    copy.set_updated_at(now);
    copy
}

/// A logged conflict with both versions decrypted, for the sync status page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictEntry {
    /// Conflict-log id (pass to `mark_conflict_reviewed` / `restore_conflict_loser`).
    pub id: i64,
    /// The item the conflict was about.
    pub item_id: String,
    pub resolution: Resolution,
    /// The local version at the time, if it was a live item that can still be decrypted.
    pub local: Option<Item>,
    /// The remote version at the time.
    pub remote: Option<Item>,
    /// Whether the local side was a deletion.
    pub local_deleted: bool,
    /// Whether the remote side was a deletion.
    pub remote_deleted: bool,
    pub local_updated_at: Option<i64>,
    pub remote_updated_at: Option<i64>,
    /// When it was resolved, Unix ms.
    pub created_at: i64,
    /// Whether the user has seen it.
    pub reviewed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Group, Host, SshKey};

    fn host(name: &str, at: i64) -> Item {
        Item::Host(Host {
            name: name.into(),
            updated_at: at,
            ..Host::default()
        })
    }

    fn key(name: &str, at: i64) -> Item {
        Item::Key(SshKey {
            name: name.into(),
            updated_at: at,
            ..SshKey::default()
        })
    }

    #[test]
    fn newer_local_wins() {
        let (l, r) = (host("l", 20), host("r", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepLocal { copy_remote: false }
        );
    }

    #[test]
    fn newer_remote_wins() {
        let (l, r) = (host("l", 10), host("r", 20));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepRemote { copy_local: false }
        );
    }

    #[test]
    fn tie_goes_to_remote() {
        let (l, r) = (host("l", 10), host("r", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepRemote { copy_local: false }
        );
    }

    #[test]
    fn identical_content_converges() {
        let (l, r) = (host("same", 10), host("same", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::Converged
        );
    }

    #[test]
    fn same_name_different_timestamp_is_still_a_conflict() {
        let (l, r) = (host("same", 11), host("same", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepLocal { copy_remote: false }
        );
    }

    #[test]
    fn keys_preserve_the_loser_whichever_side_loses() {
        let (l, r) = (key("l", 20), key("r", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepLocal { copy_remote: true }
        );
        let (l, r) = (key("l", 10), key("r", 20));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepRemote { copy_local: true }
        );
        let (l, r) = (key("l", 10), key("r", 10));
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepRemote { copy_local: true }
        );
    }

    #[test]
    fn non_key_items_never_produce_copies() {
        let (l, r) = (
            Item::Group(Group {
                name: "a".into(),
                updated_at: 5,
                ..Group::default()
            }),
            host("h", 1),
        );
        // Mixed types should not happen, but still resolves by LWW without panicking.
        assert_eq!(
            decide(&Side::live(&l), &Side::live(&r)),
            Decision::KeepLocal { copy_remote: false }
        );
    }

    #[test]
    fn delete_vs_modify_modified_wins_regardless_of_time() {
        let modified = host("m", 5);
        // Local deleted *after* the remote edit: the edit still wins.
        assert_eq!(
            decide(&Side::tombstone(1_000), &Side::live(&modified)),
            Decision::KeepRemote { copy_local: false }
        );
        assert_eq!(
            decide(&Side::live(&modified), &Side::tombstone(1_000)),
            Decision::KeepLocal { copy_remote: false }
        );
        // Keys too: a deletion never destroys a modified key, and no copy is needed.
        let k = key("k", 5);
        assert_eq!(
            decide(&Side::tombstone(9), &Side::live(&k)),
            Decision::KeepRemote { copy_local: false }
        );
        assert_eq!(
            decide(&Side::live(&k), &Side::tombstone(9)),
            Decision::KeepLocal { copy_remote: false }
        );
    }

    #[test]
    fn delete_vs_delete_converges() {
        assert_eq!(
            decide(&Side::tombstone(1), &Side::tombstone(2)),
            Decision::Converged
        );
    }

    #[test]
    fn resolution_mapping() {
        assert_eq!(Decision::Converged.resolution(), None);
        assert_eq!(
            Decision::KeepLocal { copy_remote: false }.resolution(),
            Some(Resolution::LocalWins)
        );
        assert_eq!(
            Decision::KeepLocal { copy_remote: true }.resolution(),
            Some(Resolution::LocalWinsRemoteCopied)
        );
        assert_eq!(
            Decision::KeepRemote { copy_local: false }.resolution(),
            Some(Resolution::RemoteWins)
        );
        assert_eq!(
            Decision::KeepRemote { copy_local: true }.resolution(),
            Some(Resolution::RemoteWinsLocalCopied)
        );
    }

    #[test]
    fn resolution_text_round_trips() {
        for r in [
            Resolution::LocalWins,
            Resolution::RemoteWins,
            Resolution::LocalWinsRemoteCopied,
            Resolution::RemoteWinsLocalCopied,
        ] {
            assert_eq!(Resolution::parse(r.as_str()), Some(r));
        }
        assert_eq!(Resolution::parse("nope"), None);
        assert!(Resolution::LocalWins.local_won());
        assert!(!Resolution::RemoteWinsLocalCopied.local_won());
    }

    #[test]
    fn conflict_copy_renames_and_restamps() {
        let loser = key("deploy key", 5);
        let copy = conflict_copy(&loser, DEFAULT_CONFLICT_SUFFIX, 99);
        assert_eq!(copy.display_name(), "deploy key (conflict copy)");
        assert_eq!(copy.updated_at(), 99);
        // Content other than the name and timestamp is untouched.
        let (Item::Key(a), Item::Key(b)) = (&loser, &copy) else {
            panic!("keys")
        };
        assert_eq!(a.private_key, b.private_key);
        assert_eq!(loser.updated_at(), 5);
        // Localized suffix.
        assert_eq!(
            conflict_copy(&loser, "（冲突副本）", 1).display_name(),
            "deploy key（冲突副本）"
        );
    }
}
