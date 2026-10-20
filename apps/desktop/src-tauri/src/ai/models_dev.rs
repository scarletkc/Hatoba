//! AI-03: models.dev's data, which fills in what providers' model lists leave out
//! ([`hatoba_ai::models::models_dev`]). It matters when models are set up: the form shows and
//! saves the limits and levels it fills in, and requests use what was saved. So the app fetches it
//! once a run, when a provider's model list is fetched or its connection tested, which go to the
//! network anyway; until then, or when the fetch fails, the snapshot Hatoba ships fills them in.

use std::time::Duration;

use hatoba_ai::error::AiError;
use hatoba_ai::models::models_dev;
use tauri::async_runtime::JoinHandle;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// How long fetching a model list waits for models.dev's data, so the list gets it; a slower
/// fetch goes on in the background.
pub const LIST_WAIT: Duration = Duration::from_secs(5);

/// Set once this run's fetch has succeeded.
static FETCHED: OnceCell<()> = OnceCell::const_new();

/// Fetches models.dev's data and uses it, unless this run already did. Returns the fetch, which
/// runs on whether or not it is awaited. A failed fetch leaves the data in use as it was, and the
/// next call tries again.
pub fn fetch_once(http: reqwest::Client) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let fetched = FETCHED
            .get_or_try_init(|| async {
                let data = models_dev::fetch(&http, &CancellationToken::new()).await?;
                tracing::info!(models = data.catalog.len(), "fetched models.dev's data");
                models_dev::install(data.catalog);
                Ok::<_, AiError>(())
            })
            .await;
        if let Err(e) = fetched {
            tracing::info!(
                kind = super::error_kind(&e),
                status = ?e.status(),
                "fetching models.dev's data failed"
            );
        }
    })
}
