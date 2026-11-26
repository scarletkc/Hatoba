use serde::{Deserialize, Serialize};
use specta::Type;

// ───────────────────────── Sync ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncKind {
    None,
    Worker,
    D1,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Off,
    Idle,
    Syncing,
    Offline,
    AuthFailed,
    Error,
    /// The Worker's version stops sync until it or the app is updated; `worker_update` says which.
    Paused,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SyncCounts {
    pub hosts: u32,
    pub keys: u32,
    pub groups: u32,
}

#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "sync://status")]
pub struct SyncStatus {
    pub kind: SyncKind,
    /// The Worker's host, or `api.cloudflare.com` in D1 direct mode.
    pub endpoint: Option<String>,
    /// D1 direct mode: the database's ID.
    pub database: Option<String>,
    pub state: SyncState,
    pub last_synced_at: Option<i64>,
    /// Local changes not pushed yet.
    pub pending: u32,
    /// Automatically resolved conflicts not reviewed yet (§6.4).
    pub conflicts: u32,
    pub auto_sync: bool,
    pub message: Option<String>,
    pub counts: Option<SyncCounts>,
    /// Worker mode: how the Worker's `/v1/health` compares with this build (§6.7, Upgrades).
    /// `None` when there is nothing to show, including a dismissed "update available".
    pub worker_update: Option<WorkerUpdate>,
    /// The items the last sync round could not upload because they are larger than sync accepts
    /// (§6.2). Empty while locked and until a round of this connection has finished.
    pub too_large: Vec<TooLargeItem>,
}

/// An item larger than sync accepts. It stays pending, and every round tries it again until it is
/// shortened or deleted.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TooLargeItem {
    pub item_id: String,
    pub item_type: ItemType,
    pub item_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerUpdateKind {
    /// Older than the bundled version: sync continues, and the update can be dismissed.
    Available,
    /// Older than the minimum version, or a lower `api`: sync is paused.
    Required,
    /// A newer `api` than this app speaks: sync is paused until Hatoba is updated.
    AppRequired,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct WorkerUpdate {
    pub kind: WorkerUpdateKind,
    /// The version the Worker reports.
    pub version: String,
    /// The version "Update Worker" deploys; `None` in builds without the Worker bundle.
    pub bundled: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncConfigInput {
    Worker {
        url: String,
        setup_token: Option<String>,
    },
    D1 {
        account_id: String,
        database_id: String,
        api_token: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct D1Database {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SyncTestResult {
    pub ok: bool,
    /// The remote already holds a vault.
    pub initialized: bool,
    pub version: Option<String>,
    pub latency_ms: Option<u32>,
    /// D1 direct mode: the databases the token can see, to pick from once the token is verified.
    pub databases: Option<Vec<D1Database>>,
    pub error: Option<crate::error::AppError>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub current: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    LocalWon,
    RemoteWon,
    KeptBoth,
    ModifiedWon,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ConflictField {
    /// A field key the interface translates, such as `port`, `jump_host`, `tags`, `name` or
    /// `state`.
    pub field: String,
    pub local: Option<String>,
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Host,
    TelnetHost,
    Group,
    Key,
    KnownHost,
    Forward,
    Snippet,
    Proxy,
    AiProvider,
    SearchProvider,
    AiConversation,
    AiMessage,
    Skill,
    SkillFile,
    McpServer,
    AiMemory,
    Settings,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ConflictView {
    pub id: i64,
    pub item_id: String,
    pub item_type: ItemType,
    pub item_name: String,
    pub resolution: ConflictResolution,
    pub local_updated_at: Option<i64>,
    pub remote_updated_at: Option<i64>,
    pub local_deleted: bool,
    pub remote_deleted: bool,
    /// Only the fields that differ; secrets are never included.
    pub fields: Vec<ConflictField>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    Keep,
    Restore,
}

// ───────────────────────── In-app deployment (§6.7) ─────────────────────────

#[derive(Debug, Clone, Serialize, Type)]
pub struct CloudflareAccount {
    pub id: String,
    pub name: String,
}

/// Step 1 passed. Later calls name the deployment by `handle`; the API token stays in Rust.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployStart {
    pub handle: String,
    /// The token belongs to the account given with it rather than to a user.
    pub account_owned: bool,
    /// The accounts a user token reaches; empty when the user has to enter the account ID.
    pub accounts: Vec<CloudflareAccount>,
    /// The default Worker and database names.
    pub worker_name: String,
    pub database_name: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct DeployTarget {
    pub account_id: String,
    pub worker_name: String,
    pub database_name: String,
    /// The workers.dev subdomain to create when the account has none.
    pub subdomain: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployWorkerAction {
    Create,
    Update,
    /// A Hatoba Worker whose database holds a vault: the deployment stops.
    HasVault,
    /// A Worker the app does not recognize: the deployment stops.
    Foreign,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployDatabaseAction {
    Create,
    /// An existing database with nothing but Hatoba's migrations.
    Use,
    /// The database already bound to the Hatoba Worker.
    Bound,
}

/// What step 2 found, shown before anything is written.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployPlan {
    /// The account's workers.dev subdomain; `None` means the user has to choose one.
    pub subdomain: Option<String>,
    pub worker: DeployWorkerAction,
    pub database: Option<DeployDatabaseAction>,
    pub database_name: Option<String>,
    /// For `has_vault`: the workers.dev URL to join it on (Flow C), or `None` when no workers.dev
    /// URL reaches the Worker.
    pub vault_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStep {
    Verify,
    Inspect,
    CreateDatabase,
    Migrate,
    Upload,
    SetupToken,
    Route,
    Wait,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStepStatus {
    Running,
    Done,
    Skipped,
}

/// Sent on the `deploy_run` channel as each step starts and ends.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployProgress {
    pub step: DeployStep,
    pub status: DeployStepStatus,
}

/// What the "Update Worker" form starts from (§6.7, Upgrades).
#[derive(Debug, Clone, Serialize, Type)]
pub struct UpgradeDefaults {
    /// The Worker URL in the sync settings.
    pub url: String,
    /// The account ID, when the app deployed the Worker.
    pub account_id: Option<String>,
    /// The Worker name: recorded when the app deployed it, or the first label of a workers.dev
    /// URL. `None` for a custom domain, where the user enters it.
    pub worker_name: Option<String>,
    /// The app deployed the Worker. Otherwise it may have come from the Deploy to Cloudflare
    /// button, whose next push from the user's repository replaces the upgrade.
    pub deployed_by_app: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpgradeTarget {
    pub account_id: String,
    pub worker_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeWorkerAction {
    Upgrade,
    /// No Worker has the name: the upgrade stops.
    Missing,
    /// A Worker the app does not recognize: the upgrade stops.
    Foreign,
    /// A Hatoba Worker without a vault: the upgrade stops.
    NoVault,
    /// The Worker is newer than the bundled version: the upgrade stops.
    Newer,
}

/// What step 2 of an upgrade found, shown before anything is written.
#[derive(Debug, Clone, Serialize, Type)]
pub struct UpgradePlan {
    pub worker: UpgradeWorkerAction,
    /// The database bound to the Worker.
    pub database_name: Option<String>,
    /// Migrations step 4 applies.
    pub migrations: u32,
    /// Step 7 runs: the Worker URL is its workers.dev URL.
    pub route: bool,
    /// The version the Worker URL reports, when it answers.
    pub version: Option<String>,
    /// The version the upgrade deploys.
    pub bundled: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DeployOutcome {
    pub url: String,
    /// `false`: the Worker has not answered yet (step 8 is waiting); `deploy_check` asks again.
    pub ready: bool,
}
