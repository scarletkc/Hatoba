//! In-app deployment of the sync Worker through the Cloudflare API (spec §6.7).
//!
//! A [`Deployment`] holds the API token for one deployment and runs the numbered
//! [deployment steps](Step): it checks the token, inspects the account, creates and migrates the
//! database, uploads the Worker, sets its setup token, enables its workers.dev route, and waits
//! for it to answer. Every step reads the current state before it writes, so running the steps
//! again after a failure continues the deployment, and [`Deployment::remove_created`] deletes
//! only what this deployment created.
//!
//! The API token and the setup token stay inside the `Deployment` in zeroizing memory; nothing
//! here logs them or a request body.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::sync::backend::SyncBackend;
use crate::sync::worker::WorkerBackend;

pub mod bundle;
mod cloudflare;

#[cfg(test)]
mod tests;

pub use bundle::WorkerBundle;
pub use cloudflare::Account;

/// Production API base.
pub const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
/// The table in which wrangler records applied migrations.
pub(crate) const MIGRATIONS_TABLE: &str = "d1_migrations";
/// The Worker secret that `/v1/setup` checks.
const SETUP_TOKEN_SECRET: &str = "SETUP_TOKEN";
/// The service name `/v1/health` reports.
const SERVICE_NAME: &str = "hatoba-sync";
/// How many `{db}-N` names step 2 tries before giving up.
const MAX_DATABASE_SUFFIX: u32 = 20;

/// A permission the API token needs (spec §6.7, API token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    /// Account · Workers Scripts · Edit.
    WorkersScripts,
    /// Account · D1 · Edit.
    D1,
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WorkersScripts => "Workers Scripts Edit",
            Self::D1 => "D1 Edit",
        })
    }
}

/// The deployment steps, numbered as in the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 1. Check the token and the account.
    Verify = 1,
    /// 2. Inspect the account (read only, apart from creating a missing workers.dev subdomain).
    Inspect,
    /// 3. Create the database.
    CreateDatabase,
    /// 4. Apply the migrations.
    Migrate,
    /// 5. Upload the Worker.
    Upload,
    /// 6. Set the setup token.
    SetupToken,
    /// 7. Enable workers.dev.
    Route,
    /// 8. Wait for the Worker.
    Wait,
}

/// Progress of one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    /// The step started.
    Running,
    /// The step finished.
    Done,
    /// The step had nothing to do.
    Skipped,
}

/// A step that failed, and why.
#[derive(Debug)]
pub struct StepFailure {
    /// The step.
    pub step: Step,
    /// The error.
    pub error: Error,
}

/// What [`Deployment::deploy`] ended with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The Worker answers `/v1/health` with the bundled version and no vault.
    Ready,
    /// The Worker did not answer in time, usually while a new workers.dev subdomain's DNS
    /// propagates. [`Deployment::check_ready`] tries again.
    Waiting,
}

/// The token check of step 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInfo {
    /// The token belongs to the account rather than to a user.
    pub account_owned: bool,
    /// The accounts a user token reaches. Empty for an account-owned token, and when Cloudflare
    /// does not list them.
    pub accounts: Vec<Account>,
}

/// Where to deploy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Cloudflare account ID.
    pub account_id: String,
    /// Worker name.
    pub worker_name: String,
    /// The database name to use when a new database is needed.
    pub database_name: String,
    /// The workers.dev subdomain to create when the account has none.
    pub subdomain: Option<String>,
}

impl Target {
    /// Checks the names Cloudflare will accept.
    ///
    /// # Errors
    /// [`Error::InvalidItem`] naming the field.
    pub fn validate(&self) -> Result<()> {
        if !is_identifier(&self.account_id) {
            return Err(Error::InvalidItem("account_id".into()));
        }
        if !is_dns_label(&self.worker_name) {
            return Err(Error::InvalidItem("worker_name".into()));
        }
        // Leaves room for the `-N` suffix.
        let db = &self.database_name;
        if db.is_empty()
            || db.len() > 60
            || !db
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err(Error::InvalidItem("database_name".into()));
        }
        if self.subdomain.as_deref().is_some_and(|s| !is_dns_label(s)) {
            return Err(Error::InvalidItem("subdomain".into()));
        }
        Ok(())
    }
}

/// A lowercase DNS label: letters, digits, and inner hyphens, at most 63 characters.
fn is_dns_label(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn is_identifier(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What step 2 found for the Worker name (spec §6.7, existing Workers and databases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerPlan {
    /// No Worker has the name: create it.
    Create,
    /// A Hatoba Worker whose database holds no vault: deploy over it.
    Update,
    /// A Hatoba Worker whose database holds a vault: stop (Flow C).
    HasVault,
    /// A Worker the app does not recognize: stop and ask for another name.
    Foreign,
}

/// The database the deployment uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatabasePlan {
    /// Create a database with this name.
    Create {
        /// Name.
        name: String,
    },
    /// Use an empty database, or one migrated by Hatoba without a vault.
    Use {
        /// Database ID.
        id: String,
        /// Name.
        name: String,
    },
    /// Use the database already bound to the Hatoba Worker as `DB`.
    Bound {
        /// Database ID.
        id: String,
        /// Name.
        name: String,
    },
}

impl DatabasePlan {
    /// The database name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Create { name } | Self::Use { name, .. } | Self::Bound { name, .. } => name,
        }
    }
}

/// The plan step 2 picks before anything is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    /// The account's workers.dev subdomain, or `None` when it has none yet.
    pub subdomain: Option<String>,
    /// What happens to the Worker.
    pub worker: WorkerPlan,
    /// The database, unless the Worker plan stops the deployment.
    pub database: Option<DatabasePlan>,
}

/// What a database holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatabaseState {
    /// No tables apart from SQLite's and D1's internal ones.
    Empty,
    /// Migrated by Hatoba, with no vault.
    Migrated,
    /// `meta` has the vault row.
    HasVault,
    /// Anything else.
    Other,
}

/// Health check pacing for step 8.
#[derive(Clone, Copy, Debug)]
pub struct Polling {
    /// Between checks.
    pub interval: Duration,
    /// How long step 8 waits before it reports [`Outcome::Waiting`].
    pub timeout: Duration,
}

impl Default for Polling {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3),
            timeout: Duration::from_secs(120),
        }
    }
}

/// Resources this deployment created, which cleanup may delete.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Created {
    /// `(account ID, Worker name)`.
    pub worker: Option<(String, String)>,
    /// `(account ID, database ID)`.
    pub database: Option<(String, String)>,
}

impl Created {
    /// Whether there is anything to remove.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.worker.is_none() && self.database.is_none()
    }
}

/// A deployed Worker waiting for `/v1/setup`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployed {
    /// Cloudflare account ID.
    pub account_id: String,
    /// Worker name.
    pub worker_name: String,
    /// `https://{name}.{subdomain}.workers.dev`.
    pub url: String,
}

/// One in-app deployment (spec §6.7).
pub struct Deployment {
    api: cloudflare::Client,
    bundle: Arc<WorkerBundle>,
    /// Replaces `https://{name}.{subdomain}.workers.dev` for the health checks (tests).
    worker_origin: Option<String>,
    polling: Polling,
    created: Created,
    deployed: Option<Deployed>,
    setup_token: Option<Zeroizing<String>>,
}

impl fmt::Debug for Deployment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Deployment")
            .field("created", &self.created)
            .field("deployed", &self.deployed)
            .finish_non_exhaustive()
    }
}

/// Wraps an error with the step it happened in.
fn at(step: Step) -> impl FnOnce(Error) -> StepFailure {
    move |error| StepFailure { step, error }
}

impl Deployment {
    /// A deployment of `bundle` authenticated with `api_token`.
    ///
    /// # Errors
    /// [`Error::CloudflareToken`] for an empty token.
    pub fn new(api_token: &str, bundle: Arc<WorkerBundle>) -> Result<Self> {
        Self::with_endpoints(CLOUDFLARE_API_BASE, None, api_token, bundle)
    }

    /// As [`new`](Self::new) against another API base, with the Worker's health checks sent to
    /// `worker_origin` (tests; `http` is allowed for loopback).
    ///
    /// # Errors
    /// As [`new`](Self::new), and [`Error::InvalidUrl`] for a bad API base.
    pub fn with_endpoints(
        api_base: &str,
        worker_origin: Option<&str>,
        api_token: &str,
        bundle: Arc<WorkerBundle>,
    ) -> Result<Self> {
        Ok(Self {
            api: cloudflare::Client::new(api_base, api_token)?,
            bundle,
            worker_origin: worker_origin.map(str::to_owned),
            polling: Polling::default(),
            created: Created::default(),
            deployed: None,
            setup_token: None,
        })
    }

    /// Changes the health check pacing of step 8.
    pub fn set_polling(&mut self, polling: Polling) {
        self.polling = polling;
    }

    /// The bundle this deployment uploads.
    #[must_use]
    pub fn bundle(&self) -> &WorkerBundle {
        &self.bundle
    }

    /// What this deployment created so far.
    #[must_use]
    pub fn created(&self) -> &Created {
        &self.created
    }

    /// The deployed Worker, once steps 3 to 7 succeeded.
    #[must_use]
    pub fn deployed(&self) -> Option<&Deployed> {
        self.deployed.as_ref()
    }

    /// The setup token step 6 set, which the master password step passes to `/v1/setup`.
    #[must_use]
    pub fn setup_token(&self) -> Option<&str> {
        self.setup_token.as_deref().map(String::as_str)
    }

    /// Step 1: checks the token and lists the accounts it reaches. A token that Cloudflare does
    /// not accept as a user token is checked as a token owned by `account_id`.
    ///
    /// # Errors
    /// [`Error::CloudflareToken`] when Cloudflare rejects the token or it is not active;
    /// transport errors.
    pub async fn verify(&self, account_id: Option<&str>) -> Result<TokenInfo> {
        let account_id = account_id.map(str::trim).filter(|a| !a.is_empty());
        if account_id.is_some_and(|a| !is_identifier(a)) {
            return Err(Error::InvalidItem("account_id".into()));
        }
        let account_owned = self.api.verify_token(account_id).await?;
        let accounts = if account_owned {
            Vec::new()
        } else {
            self.api.accounts().await?
        };
        Ok(TokenInfo {
            account_owned,
            accounts,
        })
    }

    /// Step 2: picks the plan for `target` without writing anything. Reads from both Workers and
    /// D1, so a missing permission shows up here.
    ///
    /// # Errors
    /// [`Error::CloudflarePermission`] naming the missing permission; other API errors.
    pub async fn inspect(&self, target: &Target) -> Result<Inspection> {
        target.validate()?;
        let account = target.account_id.as_str();
        let subdomain = self.api.subdomain(account).await?;
        let worker = self
            .api
            .script_settings(account, &target.worker_name)
            .await?;
        let (plan, bound) = match worker {
            None => (WorkerPlan::Create, None),
            Some(settings) => match self.bound_database(&settings.bindings) {
                None => {
                    return Ok(Inspection {
                        subdomain,
                        worker: WorkerPlan::Foreign,
                        database: None,
                    });
                }
                Some(id) => (WorkerPlan::Update, Some(id)),
            },
        };
        // A bound database that was deleted gets replaced by a new one.
        if let Some(id) = bound
            && let Some(db) = self.api.database(account, &id).await?
        {
            if self.database_state(account, &db.uuid).await? == DatabaseState::HasVault {
                return Ok(Inspection {
                    subdomain,
                    worker: WorkerPlan::HasVault,
                    database: None,
                });
            }
            return Ok(Inspection {
                subdomain,
                worker: plan,
                database: Some(DatabasePlan::Bound {
                    id: db.uuid,
                    name: db.name,
                }),
            });
        }
        let database = self.pick_database(account, &target.database_name).await?;
        Ok(Inspection {
            subdomain,
            worker: plan,
            database: Some(database),
        })
    }

    /// The ID of the database bound as `DB`, when the bindings are a Hatoba Worker's: a `d1`
    /// binding named `DB` and a `ratelimit` binding named `AUTH_LIMITER`.
    fn bound_database(&self, bindings: &[Value]) -> Option<String> {
        let find = |kind: &str, name: &str| {
            bindings.iter().find(|b| {
                b.get("type").and_then(Value::as_str) == Some(kind)
                    && b.get("name").and_then(Value::as_str) == Some(name)
            })
        };
        find("ratelimit", &self.bundle.ratelimit.name)?;
        let d1 = find("d1", &self.bundle.d1.binding)?;
        // `id` is the older name of `database_id`.
        d1.get("database_id")
            .or_else(|| d1.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// The first of `{db}`, `{db}-2`, `{db}-3`, … that is free or holds nothing but an earlier
    /// attempt's migrations.
    async fn pick_database(&self, account: &str, name: &str) -> Result<DatabasePlan> {
        let existing = self.api.databases(account, name).await?;
        for n in 1..=MAX_DATABASE_SUFFIX {
            let candidate = if n == 1 {
                name.to_owned()
            } else {
                format!("{name}-{n}")
            };
            // The list is a search, so only an exact match counts.
            let Some(db) = existing.iter().find(|d| d.name == candidate) else {
                return Ok(DatabasePlan::Create { name: candidate });
            };
            if matches!(
                self.database_state(account, &db.uuid).await?,
                DatabaseState::Empty | DatabaseState::Migrated
            ) {
                return Ok(DatabasePlan::Use {
                    id: db.uuid.clone(),
                    name: db.name.clone(),
                });
            }
        }
        // Every candidate holds something else: the user picks another name.
        Err(Error::InvalidItem("database_name".into()))
    }

    async fn query(&self, account: &str, database: &str, sql: &str) -> Result<Vec<Value>> {
        let statements = self
            .api
            .query(account, database, sql)
            .await
            .map_err(|f| f.into_error(Permission::D1))?;
        Ok(statements
            .into_iter()
            .next_back()
            .map(|s| s.results.into_iter().map(Value::Object).collect())
            .unwrap_or_default())
    }

    async fn column(&self, account: &str, database: &str, sql: &str) -> Result<Vec<String>> {
        Ok(self
            .query(account, database, sql)
            .await?
            .iter()
            .filter_map(|row| row.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect())
    }

    async fn database_state(&self, account: &str, database: &str) -> Result<DatabaseState> {
        let tables: Vec<String> = self
            .column(
                account,
                database,
                "SELECT name FROM sqlite_master WHERE type = 'table'",
            )
            .await?
            .into_iter()
            .map(|t| t.to_ascii_lowercase())
            .filter(|t| !t.starts_with("sqlite_") && !t.starts_with("_cf_"))
            .collect();
        if tables.is_empty() {
            return Ok(DatabaseState::Empty);
        }
        if tables.iter().any(|t| t == "meta") {
            match self
                .query(
                    account,
                    database,
                    "SELECT 1 AS present FROM meta WHERE id = 1",
                )
                .await
            {
                Ok(rows) if !rows.is_empty() => return Ok(DatabaseState::HasVault),
                Ok(_) => {}
                // D1 rejects the query (HTTP 400) for a `meta` table of some other shape.
                Err(Error::Cloudflare { status: 400, .. }) => return Ok(DatabaseState::Other),
                Err(e) => return Err(e),
            }
        }
        if !tables.iter().any(|t| t == MIGRATIONS_TABLE) {
            return Ok(DatabaseState::Other);
        }
        let applied = self
            .column(
                account,
                database,
                &format!("SELECT name FROM {MIGRATIONS_TABLE}"),
            )
            .await?;
        let bundled = |name: &String| self.bundle.migrations.iter().any(|m| &m.name == name);
        let owned = self.bundle.owned_tables();
        if applied.iter().all(bundled) && tables.iter().all(|t| owned.contains(t)) {
            Ok(DatabaseState::Migrated)
        } else {
            Ok(DatabaseState::Other)
        }
    }

    /// Steps 2 to 8. Run it again after a failure to continue the deployment.
    ///
    /// # Errors
    /// The step that failed, with [`Error::RemoteInitialized`] or [`Error::WorkerNameTaken`]
    /// when step 2 finds a Worker it must not deploy over, [`Error::SubdomainRequired`] when the
    /// account has no workers.dev subdomain and `target` names none, or the API error.
    pub async fn deploy(
        &mut self,
        target: &Target,
        progress: &(dyn Fn(Step, StepStatus) + Send + Sync),
    ) -> std::result::Result<Outcome, StepFailure> {
        let account = target.account_id.clone();
        let name = target.worker_name.clone();

        progress(Step::Inspect, StepStatus::Running);
        let inspection = self.inspect(target).await.map_err(at(Step::Inspect))?;
        let database = match (inspection.worker.clone(), inspection.database) {
            (WorkerPlan::HasVault, _) => return Err(at(Step::Inspect)(Error::RemoteInitialized)),
            (WorkerPlan::Foreign, _) | (_, None) => {
                return Err(at(Step::Inspect)(Error::WorkerNameTaken));
            }
            (_, Some(database)) => database,
        };
        let subdomain = match inspection.subdomain {
            Some(s) => s,
            None => {
                let wanted = target
                    .subdomain
                    .as_deref()
                    .ok_or(Error::SubdomainRequired)
                    .map_err(at(Step::Inspect))?;
                self.api
                    .create_subdomain(&account, wanted)
                    .await
                    .map_err(at(Step::Inspect))?
            }
        };
        progress(Step::Inspect, StepStatus::Done);

        let database_id = match database {
            DatabasePlan::Create { name } => {
                progress(Step::CreateDatabase, StepStatus::Running);
                let db = self
                    .api
                    .create_database(&account, &name)
                    .await
                    .map_err(at(Step::CreateDatabase))?;
                self.created.database = Some((account.clone(), db.uuid.clone()));
                progress(Step::CreateDatabase, StepStatus::Done);
                db.uuid
            }
            DatabasePlan::Use { id, .. } | DatabasePlan::Bound { id, .. } => {
                progress(Step::CreateDatabase, StepStatus::Skipped);
                id
            }
        };

        progress(Step::Migrate, StepStatus::Running);
        let applied = self
            .migrate(&account, &database_id)
            .await
            .map_err(at(Step::Migrate))?;
        progress(
            Step::Migrate,
            if applied == 0 {
                StepStatus::Skipped
            } else {
                StepStatus::Done
            },
        );

        progress(Step::Upload, StepStatus::Running);
        if inspection.worker == WorkerPlan::Create {
            // Recorded first: a timed-out upload may still have created it.
            self.created.worker = Some((account.clone(), name.clone()));
        }
        self.api
            .upload(&account, &name, &self.bundle, &database_id)
            .await
            .map_err(at(Step::Upload))?;
        progress(Step::Upload, StepStatus::Done);

        progress(Step::SetupToken, StepStatus::Running);
        let token = new_setup_token().map_err(at(Step::SetupToken))?;
        self.api
            .put_secret(&account, &name, SETUP_TOKEN_SECRET, &token)
            .await
            .map_err(at(Step::SetupToken))?;
        self.setup_token = Some(token);
        progress(Step::SetupToken, StepStatus::Done);

        progress(Step::Route, StepStatus::Running);
        self.api
            .enable_route(&account, &name)
            .await
            .map_err(at(Step::Route))?;
        self.deployed = Some(Deployed {
            url: format!("https://{name}.{subdomain}.workers.dev"),
            account_id: account,
            worker_name: name,
        });
        progress(Step::Route, StepStatus::Done);

        progress(Step::Wait, StepStatus::Running);
        if self.check_ready().await.map_err(at(Step::Wait))? {
            progress(Step::Wait, StepStatus::Done);
            Ok(Outcome::Ready)
        } else {
            Ok(Outcome::Waiting)
        }
    }

    /// Step 4: applies the bundled migrations that `d1_migrations` does not list, each in one
    /// batch with its record, as `wrangler d1 migrations apply` does. Returns how many it applied.
    async fn migrate(&self, account: &str, database: &str) -> Result<usize> {
        // wrangler's statement, so both tools share the table.
        self.query(
            account,
            database,
            &format!(
                "CREATE TABLE IF NOT EXISTS \"{MIGRATIONS_TABLE}\"(\n\t\tid         INTEGER PRIMARY KEY AUTOINCREMENT,\n\t\tname       TEXT UNIQUE,\n\t\tapplied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL\n);"
            ),
        )
        .await?;
        let applied = self
            .column(
                account,
                database,
                &format!("SELECT name FROM \"{MIGRATIONS_TABLE}\" ORDER BY id"),
            )
            .await?;
        let mut count = 0;
        for migration in &self.bundle.migrations {
            if applied.contains(&migration.name) {
                continue;
            }
            let sql = format!(
                "{}\nINSERT INTO \"{MIGRATIONS_TABLE}\" (name)\nvalues ('{}');",
                migration.sql,
                migration.name.replace('\'', "''")
            );
            self.query(account, database, &sql).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Step 8: polls the Worker's `/v1/health` until it reports the bundled version with no
    /// vault, for up to [`Polling::timeout`]. `false` means it has not answered yet.
    ///
    /// # Errors
    /// [`Error::RemoteInitialized`] when the Worker already holds a vault;
    /// [`Error::Protocol`] before [`deploy`](Self::deploy) reached step 7.
    pub async fn check_ready(&self) -> Result<bool> {
        let deployed = self
            .deployed
            .as_ref()
            .ok_or_else(|| Error::Protocol("the Worker is not deployed yet".into()))?;
        let backend = WorkerBackend::new(self.worker_origin.as_deref().unwrap_or(&deployed.url))?;
        let deadline = tokio::time::Instant::now() + self.polling.timeout;
        loop {
            if let Ok(info) = backend.health().await
                && info.service == SERVICE_NAME
                && info.version == self.bundle.version
            {
                if info.initialized {
                    return Err(Error::RemoteInitialized);
                }
                return Ok(true);
            }
            if tokio::time::Instant::now() + self.polling.interval > deadline {
                return Ok(false);
            }
            tokio::time::sleep(self.polling.interval).await;
        }
    }

    /// After `/v1/setup` succeeded: deletes the `SETUP_TOKEN` secret, so `/v1/setup` answers
    /// `503 setup_token_not_configured` from then on, and forgets the setup token.
    ///
    /// # Errors
    /// API errors. The vault is initialized either way, so a failure is only worth logging.
    pub async fn finish(&mut self) -> Result<()> {
        self.setup_token = None;
        let Some(deployed) = &self.deployed else {
            return Ok(());
        };
        self.api
            .delete_secret(
                &deployed.account_id,
                &deployed.worker_name,
                SETUP_TOKEN_SECRET,
            )
            .await
    }

    /// "Remove what Hatoba created": deletes, in reverse order, the Worker and the database this
    /// deployment created. It never deletes one that existed before.
    ///
    /// # Errors
    /// API errors; what was already deleted stays forgotten, so calling it again continues.
    pub async fn remove_created(&mut self) -> Result<()> {
        self.deployed = None;
        self.setup_token = None;
        if let Some((account, name)) = self.created.worker.clone() {
            self.api.delete_script(&account, &name).await?;
            self.created.worker = None;
        }
        if let Some((account, id)) = self.created.database.clone() {
            self.api.delete_database(&account, &id).await?;
            self.created.database = None;
        }
        Ok(())
    }
}

/// 32 random bytes as base64url, like a session token (spec §6.2).
fn new_setup_token() -> Result<Zeroizing<String>> {
    let bytes = Zeroizing::new(crate::crypto::random_bytes::<32>()?);
    Ok(Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(*bytes),
    ))
}
