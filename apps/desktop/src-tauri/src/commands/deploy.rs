//! In-app deployment of the sync Worker (spec §6.7, DEPLOY-01…07), and its upgrades (DEPLOY-08).

use std::sync::Arc;

use hatoba_core::sync::deploy::{
    self as core_deploy, DatabasePlan, Deployment, Outcome, Step, StepFailure, StepStatus, Target,
    UpgradeWorker, WorkerPlan,
};
use hatoba_core::sync::{
    SyncBackend, SyncConfig, WorkerBackend, WorkerDeployment, flows, save_session,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::deploy::bundle;
use crate::dto::{
    CloudflareAccount, DeployDatabaseAction, DeployOutcome, DeployPlan, DeployProgress,
    DeployStart, DeployStep, DeployStepStatus, DeployTarget, DeployWorkerAction, UpgradeDefaults,
    UpgradePlan, UpgradeTarget, UpgradeWorkerAction,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::AppState;
use crate::sync::{self, Trigger, device_info};

/// Names the field of an invalid target, so the wizard can mark it.
fn deploy_error(e: hatoba_core::Error) -> AppError {
    match e {
        hatoba_core::Error::InvalidItem(field) => AppError::invalid(&field, "invalid name"),
        other => other.into(),
    }
}

fn target(input: DeployTarget) -> Target {
    Target {
        account_id: input.account_id.trim().to_owned(),
        worker_name: input.worker_name.trim().to_owned(),
        database_name: input.database_name.trim().to_owned(),
        subdomain: input
            .subdomain
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    }
}

fn step(step: Step) -> DeployStep {
    match step {
        Step::Verify => DeployStep::Verify,
        Step::Inspect => DeployStep::Inspect,
        Step::CreateDatabase => DeployStep::CreateDatabase,
        Step::Migrate => DeployStep::Migrate,
        Step::Upload => DeployStep::Upload,
        Step::SetupToken => DeployStep::SetupToken,
        Step::Route => DeployStep::Route,
        Step::Wait => DeployStep::Wait,
    }
}

/// Sends each step's progress to the wizard.
fn reporter(progress: &Channel<DeployProgress>) -> impl Fn(Step, StepStatus) + Send + Sync + '_ {
    move |s, status| {
        tracing::info!(step = s as u8, ?status, "deploy step");
        let _ = progress.send(DeployProgress {
            step: step(s),
            status: match status {
                StepStatus::Running => DeployStepStatus::Running,
                StepStatus::Done => DeployStepStatus::Done,
                StepStatus::Skipped => DeployStepStatus::Skipped,
            },
        });
    }
}

fn step_failed(failure: StepFailure) -> AppError {
    // The error's text has the HTTP status and Cloudflare's code, never a body or token.
    tracing::warn!(
        step = failure.step as u8,
        code = failure.error.code(),
        "deploy step failed: {}",
        failure.error
    );
    deploy_error(failure.error)
}

/// Step 1 (DEPLOY-02): checks the API token. The token crosses to Rust here once; every later
/// call names the deployment by the returned handle (DEPLOY-07).
#[tauri::command]
#[specta::specta]
pub async fn deploy_start(
    state: State<'_, AppState>,
    api_token: String,
    account_id: Option<String>,
) -> AppResult<DeployStart> {
    let api_token = Zeroizing::new(api_token);
    if !state.vault().is_unlocked() {
        return Err(AppError::locked());
    }
    let bundle = bundle()?;
    let deployment = Deployment::new(&api_token, Arc::clone(&bundle)).map_err(deploy_error)?;
    let info = deployment
        .verify(account_id.as_deref())
        .await
        .map_err(deploy_error)?;
    tracing::info!(
        account_owned = info.account_owned,
        accounts = info.accounts.len(),
        "deploy: token checked"
    );
    Ok(DeployStart {
        handle: state.deploy.start(deployment),
        account_owned: info.account_owned,
        accounts: info
            .accounts
            .into_iter()
            .map(|a| CloudflareAccount {
                id: a.id,
                name: a.name,
            })
            .collect(),
        worker_name: bundle.name.clone(),
        database_name: bundle.d1.database_name.clone(),
    })
}

/// Step 2: what the deployment will do, before anything is written.
#[tauri::command]
#[specta::specta]
pub async fn deploy_inspect(
    state: State<'_, AppState>,
    handle: String,
    target: DeployTarget,
) -> AppResult<DeployPlan> {
    let deployment = state.deploy.get(&handle)?;
    let deployment = deployment.lock().await;
    let plan = deployment
        .inspect(&self::target(target))
        .await
        .map_err(deploy_error)?;
    let (database, database_name) = match plan.database {
        None => (None, None),
        Some(db) => {
            let action = match &db {
                DatabasePlan::Create { .. } => DeployDatabaseAction::Create,
                DatabasePlan::Use { .. } => DeployDatabaseAction::Use,
                DatabasePlan::Bound { .. } => DeployDatabaseAction::Bound,
            };
            (Some(action), Some(db.name().to_owned()))
        }
    };
    Ok(DeployPlan {
        subdomain: plan.subdomain,
        worker: match plan.worker {
            WorkerPlan::Create => DeployWorkerAction::Create,
            WorkerPlan::Update => DeployWorkerAction::Update,
            WorkerPlan::HasVault => DeployWorkerAction::HasVault,
            WorkerPlan::Foreign => DeployWorkerAction::Foreign,
        },
        database,
        database_name,
    })
}

/// Steps 2 to 8 (DEPLOY-03), reporting each step on `progress`. Run it again after a failure to
/// continue the deployment (DEPLOY-05).
#[tauri::command]
#[specta::specta]
pub async fn deploy_run(
    state: State<'_, AppState>,
    handle: String,
    target: DeployTarget,
    progress: Channel<DeployProgress>,
) -> AppResult<DeployOutcome> {
    let deployment = state.deploy.get(&handle)?;
    let mut deployment = deployment.lock().await;
    let outcome = deployment
        .deploy(&self::target(target), &reporter(&progress))
        .await
        .map_err(step_failed)?;
    Ok(DeployOutcome {
        url: deployment
            .deployed()
            .map(|d| d.url.clone())
            .unwrap_or_default(),
        ready: outcome == Outcome::Ready,
    })
}

/// "Check again" while step 8 waits for the Worker.
#[tauri::command]
#[specta::specta]
pub async fn deploy_check(
    app: AppHandle,
    state: State<'_, AppState>,
    handle: String,
) -> AppResult<bool> {
    let deployment = state.deploy.get(&handle)?;
    let deployment = deployment.lock().await;
    let ready = deployment.check_ready().await.map_err(deploy_error)?;
    if ready && deployment.is_upgrade() {
        upgraded(&app);
    }
    Ok(ready)
}

/// "Remove what Hatoba created": only what this deployment created (DEPLOY-05).
#[tauri::command]
#[specta::specta]
pub async fn deploy_cleanup(state: State<'_, AppState>, handle: String) -> AppResult<()> {
    let deployment = state.deploy.get(&handle)?;
    let mut deployment = deployment.lock().await;
    deployment.remove_created().await.map_err(|e| {
        tracing::warn!(code = e.code(), "deploy cleanup failed: {e}");
        deploy_error(e)
    })
}

/// Ends the deployment and forgets its tokens (the user cancelled or left the wizard).
#[tauri::command]
#[specta::specta]
pub fn deploy_cancel(state: State<'_, AppState>, handle: String) {
    state.deploy.end(&handle);
}

/// The master password step after a deployment: Flow A step 3 (§6.6) with the Worker URL and the
/// setup token from the deployment, then deletes the `SETUP_TOKEN` secret (DEPLOY-06) and ends
/// the deployment.
#[tauri::command]
#[specta::specta]
pub async fn deploy_setup(
    app: AppHandle,
    state: State<'_, AppState>,
    handle: String,
    password: String,
) -> AppResult<()> {
    let deployment = state.deploy.get(&handle)?;
    let mut deployment = deployment.lock().await;
    let deployed = deployment
        .deployed()
        .cloned()
        .ok_or_else(|| AppError::internal("the Worker is not deployed yet"))?;
    let setup_token = Zeroizing::new(deployment.setup_token().unwrap_or_default().to_owned());
    let backend = WorkerBackend::new(&deployed.url)?;
    let result = flows::enable_sync(
        &state.vault,
        &backend,
        &password,
        Some(setup_token.as_str()),
        device_info(),
    )
    .await;
    // Once /v1/setup succeeded the secret is no longer needed, even if a later step failed.
    let initialized = match &result {
        Ok(_) => true,
        Err(_) => backend.health().await.is_ok_and(|info| info.initialized),
    };
    if initialized && let Err(e) = deployment.finish().await {
        // /v1/setup cannot change an initialized vault, so this does not fail the setup.
        tracing::warn!(
            code = e.code(),
            "could not delete the setup token secret: {e}"
        );
    }
    let session = result?;

    save_session(state.secrets.as_ref(), &session)?;
    state.vault().set_sync_config(Some(&SyncConfig::Worker {
        url: backend.base_url().to_owned(),
        deployment: Some(WorkerDeployment {
            account_id: deployed.account_id,
            worker_name: deployed.worker_name,
        }),
    }))?;
    state.sync.set_backend(Some(Arc::new(backend)));
    drop(deployment);
    state.deploy.end(&handle);
    sync::emit_status(&app);
    sync::trigger(&app, Trigger::Manual);
    Ok(())
}

/// `https://{name}.{subdomain}.workers.dev` → `name`.
fn workers_dev_name(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let mut labels = host.strip_suffix(".workers.dev")?.split('.');
    let (name, _subdomain) = (labels.next()?, labels.next()?);
    labels.next().is_none().then(|| name.to_owned())
}

/// The Worker URL in the sync settings, which an upgrade polls.
fn configured_url(state: &AppState) -> AppResult<(String, Option<WorkerDeployment>)> {
    match state.vault().sync_config()? {
        Some(SyncConfig::Worker { url, deployment }) => Ok((url, deployment)),
        _ => Err(AppError::new(ErrorCode::Sync, "sync does not use a Worker")),
    }
}

fn upgrade_target(state: &AppState, input: UpgradeTarget) -> AppResult<core_deploy::UpgradeTarget> {
    Ok(core_deploy::UpgradeTarget {
        account_id: input.account_id.trim().to_owned(),
        worker_name: input.worker_name.trim().to_owned(),
        url: configured_url(state)?.0,
    })
}

/// Once the Worker answers with the bundled version, sync rereads it, resumes if the old version
/// paused it, and pushes what waited.
fn upgraded(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { sync::recheck_worker(&app).await });
}

/// What "Update Worker" starts from (DEPLOY-08): the account ID and Worker name recorded when the
/// app deployed the Worker, or else the name in its workers.dev URL.
#[tauri::command]
#[specta::specta]
pub fn deploy_upgrade_defaults(state: State<'_, AppState>) -> AppResult<UpgradeDefaults> {
    let (url, deployment) = configured_url(&state)?;
    Ok(match deployment {
        Some(d) => UpgradeDefaults {
            url,
            account_id: Some(d.account_id),
            worker_name: Some(d.worker_name),
            deployed_by_app: true,
        },
        None => UpgradeDefaults {
            worker_name: workers_dev_name(&url),
            url,
            account_id: None,
            deployed_by_app: false,
        },
    })
}

/// Step 2 of an upgrade: what it will do, before anything is written.
#[tauri::command]
#[specta::specta]
pub async fn deploy_upgrade_inspect(
    state: State<'_, AppState>,
    handle: String,
    target: UpgradeTarget,
) -> AppResult<UpgradePlan> {
    let target = upgrade_target(&state, target)?;
    let deployment = state.deploy.get(&handle)?;
    let deployment = deployment.lock().await;
    let plan = deployment
        .inspect_upgrade(&target)
        .await
        .map_err(deploy_error)?;
    let (worker, database_name) = match plan.worker {
        UpgradeWorker::Ready { database_name, .. } => {
            (UpgradeWorkerAction::Upgrade, Some(database_name))
        }
        UpgradeWorker::Missing => (UpgradeWorkerAction::Missing, None),
        UpgradeWorker::Foreign => (UpgradeWorkerAction::Foreign, None),
        UpgradeWorker::NoVault => (UpgradeWorkerAction::NoVault, None),
        UpgradeWorker::Newer => (UpgradeWorkerAction::Newer, None),
    };
    Ok(UpgradePlan {
        worker,
        database_name,
        migrations: u32::try_from(plan.migrations).unwrap_or(u32::MAX),
        route: plan.route,
        version: plan.version,
        bundled: deployment.bundle().version.clone(),
    })
}

/// "Update Worker" (DEPLOY-08): the deployment steps as an upgrade, reporting each step on
/// `progress`. Run it again after a failure to continue the upgrade.
#[tauri::command]
#[specta::specta]
pub async fn deploy_upgrade(
    app: AppHandle,
    state: State<'_, AppState>,
    handle: String,
    target: UpgradeTarget,
    progress: Channel<DeployProgress>,
) -> AppResult<DeployOutcome> {
    let target = upgrade_target(&state, target)?;
    let deployment = state.deploy.get(&handle)?;
    let mut deployment = deployment.lock().await;
    let outcome = deployment
        .upgrade(&target, &reporter(&progress))
        .await
        .map_err(step_failed)?;
    let ready = outcome == Outcome::Ready;
    if ready {
        upgraded(&app);
    }
    Ok(DeployOutcome {
        url: target.url,
        ready,
    })
}

#[cfg(test)]
mod tests {
    use super::workers_dev_name;

    #[test]
    fn takes_the_worker_name_from_a_workers_dev_url() {
        assert_eq!(
            workers_dev_name("https://hatoba-sync.kc.workers.dev").as_deref(),
            Some("hatoba-sync")
        );
        assert_eq!(
            workers_dev_name("https://My-Sync.KC.workers.dev").as_deref(),
            Some("my-sync")
        );
        for other in [
            "https://sync.example.com",
            "https://kc.workers.dev",
            "https://a.b.kc.workers.dev",
            "not a url",
        ] {
            assert_eq!(workers_dev_name(other), None, "{other}");
        }
    }
}
