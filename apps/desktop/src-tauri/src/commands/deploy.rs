//! In-app deployment of the sync Worker (spec §6.7, DEPLOY-01…07).

use std::sync::Arc;

use hatoba_core::sync::deploy::{
    DatabasePlan, Deployment, Outcome, Step, StepStatus, Target, WorkerPlan,
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
    DeployStart, DeployStep, DeployStepStatus, DeployTarget, DeployWorkerAction,
};
use crate::error::{AppError, AppResult};
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
    let report = |s: Step, status: StepStatus| {
        tracing::info!(step = s as u8, ?status, "deploy step");
        let _ = progress.send(DeployProgress {
            step: step(s),
            status: match status {
                StepStatus::Running => DeployStepStatus::Running,
                StepStatus::Done => DeployStepStatus::Done,
                StepStatus::Skipped => DeployStepStatus::Skipped,
            },
        });
    };
    match deployment.deploy(&self::target(target), &report).await {
        Ok(outcome) => Ok(DeployOutcome {
            url: deployment
                .deployed()
                .map(|d| d.url.clone())
                .unwrap_or_default(),
            ready: outcome == Outcome::Ready,
        }),
        Err(failure) => {
            // The error's text has the HTTP status and Cloudflare's code, never a body or token.
            tracing::warn!(
                step = failure.step as u8,
                code = failure.error.code(),
                "deploy step failed: {}",
                failure.error
            );
            Err(deploy_error(failure.error))
        }
    }
}

/// "Check again" while step 8 waits for the Worker.
#[tauri::command]
#[specta::specta]
pub async fn deploy_check(state: State<'_, AppState>, handle: String) -> AppResult<bool> {
    let deployment = state.deploy.get(&handle)?;
    let deployment = deployment.lock().await;
    deployment.check_ready().await.map_err(deploy_error)
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
