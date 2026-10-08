//! Which Worker versions this build syncs with (spec §6.7, Upgrades).
//!
//! The app knows two Worker versions: the bundled one, which is the newest it can deploy
//! ([`WorkerBundle::version`](super::deploy::WorkerBundle)), and [`MIN_WORKER_VERSION`], the
//! oldest it syncs with. [`worker_compat`] compares a Worker's `/v1/health` with both.

use crate::sync::backend::ServerInfo;
use crate::version::Version;

/// The Worker API this build speaks: `api` in `/v1/health`.
pub const WORKER_API: u32 = 1;

/// The oldest Worker version this build syncs with. Raise it when the app starts to rely on
/// something older Workers lack; Workers below it pause sync until they are updated.
pub const MIN_WORKER_VERSION: &str = "0.1.0";

/// How a Worker's `/v1/health` compares with this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerCompat {
    /// At or above the bundled version, so there is nothing to do. The app never deploys its
    /// bundle over a newer Worker, which another device's newer app may have deployed.
    Current,
    /// Below the bundled version but at or above the minimum: sync continues, and the sync
    /// status page offers the update.
    UpdateAvailable,
    /// A lower `api`, or a version below the minimum: sync pauses until the Worker is updated.
    UpdateRequired,
    /// A higher `api` than this build speaks: sync pauses until Hatoba is updated.
    AppUpdateRequired,
}

impl WorkerCompat {
    /// Whether sync pauses.
    #[must_use]
    pub fn pauses_sync(self) -> bool {
        matches!(self, Self::UpdateRequired | Self::AppUpdateRequired)
    }
}

/// Compares `/v1/health` with this build. `bundled` is the Worker version this build deploys,
/// or `None` in a build without the Worker bundle, which then never offers an update it could
/// not deploy.
#[must_use]
pub fn worker_compat(info: &ServerInfo, bundled: Option<&str>) -> WorkerCompat {
    if info.api > WORKER_API {
        return WorkerCompat::AppUpdateRequired;
    }
    if info.api < WORKER_API {
        return WorkerCompat::UpdateRequired;
    }
    // A Worker whose version is outside the release format was built by hand; leave it alone.
    let Some(version) = Version::parse(&info.version) else {
        return WorkerCompat::Current;
    };
    let minimum =
        Version::parse(MIN_WORKER_VERSION).expect("MIN_WORKER_VERSION is a release version");
    if version < minimum {
        return WorkerCompat::UpdateRequired;
    }
    match bundled.and_then(Version::parse) {
        Some(bundled) if version < bundled => WorkerCompat::UpdateAvailable,
        _ => WorkerCompat::Current,
    }
}

/// Whether the Worker at `version` is newer than `bundled`, so deploying the bundle over it would
/// downgrade it. A version outside the release format counts as newer.
#[must_use]
pub fn is_newer(version: &str, bundled: &str) -> bool {
    match (Version::parse(version), Version::parse(bundled)) {
        (Some(version), Some(bundled)) => version > bundled,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(api: u32, version: &str) -> ServerInfo {
        ServerInfo {
            service: "hatoba-sync".into(),
            version: version.into(),
            api,
            initialized: true,
        }
    }

    #[test]
    fn the_minimum_is_a_release_version() {
        assert!(Version::parse(MIN_WORKER_VERSION).is_some());
    }

    #[test]
    fn follows_the_upgrades_table() {
        use WorkerCompat::*;
        let min = MIN_WORKER_VERSION;
        for (info, bundled, expected) in [
            (
                health(WORKER_API + 1, "9.0.0"),
                Some("0.3.0"),
                AppUpdateRequired,
            ),
            (
                health(WORKER_API + 1, min),
                Some("0.3.0"),
                AppUpdateRequired,
            ),
            (health(WORKER_API, "0.3.0"), Some("0.3.0"), Current),
            (health(WORKER_API, "0.4.0"), Some("0.3.0"), Current),
            (
                health(WORKER_API, "0.3.0-rc.1"),
                Some("0.3.0"),
                UpdateAvailable,
            ),
            (health(WORKER_API, min), Some("99.0.0"), UpdateAvailable),
            (health(WORKER_API, "0.0.9"), Some("0.3.0"), UpdateRequired),
            (
                health(WORKER_API - 1, "0.3.0"),
                Some("0.3.0"),
                UpdateRequired,
            ),
            // Without a bundle there is nothing to offer, but the minimum still applies.
            (health(WORKER_API, min), None, Current),
            (health(WORKER_API, "0.0.9"), None, UpdateRequired),
            (health(WORKER_API, "dev"), Some("0.3.0"), Current),
        ] {
            assert_eq!(
                worker_compat(&info, bundled),
                expected,
                "{info:?} vs {bundled:?}"
            );
        }
        assert!(UpdateRequired.pauses_sync() && AppUpdateRequired.pauses_sync());
        assert!(!UpdateAvailable.pauses_sync() && !Current.pauses_sync());
    }

    #[test]
    fn a_newer_worker_is_never_downgraded() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("dev", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.3.0-rc.1", "0.3.0"));
    }
}
