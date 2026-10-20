-- Hatoba update counter: one row per day, version and platform, with the number of update checks.
-- day is YYYY-MM-DD in UTC. Nothing about a single check is stored.

CREATE TABLE checks (
  day     TEXT NOT NULL,
  version TEXT NOT NULL,
  os      TEXT NOT NULL,      -- windows, linux or darwin
  arch    TEXT NOT NULL,      -- x86_64, aarch64, ...
  bundle  TEXT NOT NULL,      -- nsis, deb, appimage, app, ...
  count   INTEGER NOT NULL,
  PRIMARY KEY (day, version, os, arch, bundle)
) WITHOUT ROWID;
