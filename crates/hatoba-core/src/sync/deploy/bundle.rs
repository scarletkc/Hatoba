//! The Worker package that `scripts/worker/bundle.mjs` builds and the shell embeds (spec §6.7,
//! Worker bundle).

use serde::Deserialize;

use crate::error::{Error, Result};

/// The package format this build reads.
const FORMAT: u32 = 1;

/// The Worker code, its migrations, and the settings from `workers/sync/wrangler.toml`.
#[derive(Clone, Debug, Deserialize)]
pub struct WorkerBundle {
    format: u32,
    /// The Worker version, which `/v1/health` reports.
    pub version: String,
    /// The default Worker name.
    pub name: String,
    /// `compatibility_date` from `wrangler.toml`.
    pub compatibility_date: String,
    /// `compatibility_flags` from `wrangler.toml`.
    #[serde(default)]
    pub compatibility_flags: Vec<String>,
    /// The single ES module that `wrangler deploy --dry-run` builds.
    pub module: Module,
    /// The D1 migrations, in file-name order.
    pub migrations: Vec<Migration>,
    /// The `DB` binding.
    pub d1: D1Binding,
    /// The `AUTH_LIMITER` binding.
    pub ratelimit: RateLimitBinding,
}

/// A JavaScript module.
#[derive(Clone, Debug, Deserialize)]
pub struct Module {
    /// File name, which is also the upload's part name.
    pub name: String,
    /// Source.
    pub content: String,
}

/// A D1 migration. Its file name is its name in `d1_migrations`.
#[derive(Clone, Debug, Deserialize)]
pub struct Migration {
    /// File name.
    pub name: String,
    /// SQL.
    pub sql: String,
}

/// The D1 binding.
#[derive(Clone, Debug, Deserialize)]
pub struct D1Binding {
    /// Binding name.
    pub binding: String,
    /// The default database name.
    pub database_name: String,
}

/// The rate limit binding.
#[derive(Clone, Debug, Deserialize)]
pub struct RateLimitBinding {
    /// Binding name.
    pub name: String,
    /// Namespace, unique within the account.
    pub namespace_id: String,
    /// Requests allowed per period.
    pub limit: u32,
    /// Period in seconds.
    pub period: u32,
}

impl WorkerBundle {
    /// Parses an embedded package.
    ///
    /// # Errors
    /// [`Error::Format`] when the package is malformed, or [`Error::UnsupportedVersion`] for a
    /// package format this build does not read.
    pub fn parse(json: &str) -> Result<Self> {
        let bundle: Self =
            serde_json::from_str(json).map_err(|_| Error::Format("worker bundle".into()))?;
        if bundle.format != FORMAT {
            return Err(Error::UnsupportedVersion(format!(
                "worker bundle format {}",
                bundle.format
            )));
        }
        if bundle.migrations.is_empty() || bundle.module.content.is_empty() {
            return Err(Error::Format("worker bundle".into()));
        }
        Ok(bundle)
    }

    /// Tables that the bundled migrations create, plus `d1_migrations`. A database whose tables
    /// are all among these, and whose recorded migrations are all bundled, was migrated by Hatoba.
    pub(crate) fn owned_tables(&self) -> Vec<String> {
        let mut tables = vec![super::MIGRATIONS_TABLE.to_owned()];
        for migration in &self.migrations {
            tables.extend(created_tables(&migration.sql));
        }
        tables
    }
}

/// Table names from `CREATE TABLE [IF NOT EXISTS] name` statements.
fn created_tables(sql: &str) -> Vec<String> {
    let words: Vec<String> = sql
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut tables = Vec::new();
    for (i, pair) in words.windows(2).enumerate() {
        if pair[0] != "create" || pair[1] != "table" {
            continue;
        }
        let mut rest = words[i + 2..].iter();
        let mut name = rest.next();
        if name.is_some_and(|w| w == "if") {
            name = rest.nth(2); // `not exists name`
        }
        if let Some(name) = name {
            tables.push(name.trim_matches(['"', '`', '[', ']']).to_owned());
        }
    }
    tables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_created_tables() {
        let sql = "-- comment\nCREATE TABLE meta (\n id INTEGER);\ncreate table if not exists \"Items\"(x);\nCREATE INDEX idx ON meta(id);\nCREATE TABLE sessions(a)";
        assert_eq!(created_tables(sql), ["meta", "items", "sessions"]);
    }
}
