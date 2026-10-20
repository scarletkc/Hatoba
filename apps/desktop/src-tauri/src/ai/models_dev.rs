//! AI-03: models.dev's data, which fills in what providers' model lists leave out
//! ([`hatoba_ai::models::models_dev`]). It matters when models are set up: the form shows and
//! saves the limits and levels it fills in, and requests use what was saved. So the app fetches it
//! each time a provider's model list is fetched, which goes to the network anyway and is how the
//! user picks up new models; until then, or when the fetch fails, the snapshot Hatoba ships fills
//! them in.

use std::time::Duration;

use hatoba_ai::models::models_dev;
use tauri::async_runtime::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How long fetching a model list waits for models.dev's data, so the list gets it; a slower
/// fetch goes on in the background.
pub const LIST_WAIT: Duration = Duration::from_secs(5);

/// Fetches models.dev's data and uses it. Returns the fetch, which runs on when the caller stops
/// waiting. A failed fetch leaves the data in use as it was.
pub fn refresh(http: reqwest::Client) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        match models_dev::fetch(&http, &CancellationToken::new()).await {
            Ok(data) => {
                tracing::info!(models = data.catalog.len(), "fetched models.dev's data");
                models_dev::install(data.catalog);
            }
            Err(e) => tracing::info!(
                kind = super::error_kind(&e),
                status = ?e.status(),
                "fetching models.dev's data failed"
            ),
        }
    })
}
