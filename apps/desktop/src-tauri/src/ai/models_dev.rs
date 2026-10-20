//! AI-03: models.dev's data, which fills in what providers' model lists leave out
//! ([`hatoba_ai::models::models_dev`]). The snapshot Hatoba ships knows the models released
//! before the version, and a released model's limits and levels do not change, so models.dev is
//! asked only about a model a fetched list leaves incomplete that the data in use does not know:
//! one newer than the snapshot, or one models.dev does not have. Once a fetch answered for a model,
//! the run does not ask about it again; after a failed fetch, the next list does. The form saves
//! what is filled in, and requests use what was saved.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hatoba_ai::models::{ModelInfo, models_dev};
use hatoba_ai::provider::Protocol;
use tokio_util::sync::CancellationToken;

/// How long a model list waits for models.dev's data; a slower fetch goes on in the background.
const LIST_WAIT: Duration = Duration::from_secs(5);

/// The model ids a fetch of this run asked models.dev about.
static ASKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
/// Held while models.dev's data is fetched, so fetches run one at a time and an older one never
/// replaces newer data.
static FETCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn asked() -> MutexGuard<'static, BTreeSet<String>> {
    ASKED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The models of `models` to ask models.dev about: those the list leaves without a limit or
/// levels that `known` does not know and no fetch of this run asked about.
fn to_ask(
    models: &[ModelInfo],
    known: impl Fn(&str) -> bool,
    asked: &BTreeSet<String>,
) -> Vec<String> {
    models
        .iter()
        .filter(|m| {
            m.context_window.is_none() || m.max_output_tokens.is_none() || m.efforts.is_none()
        })
        .filter(|m| !known(&m.id) && !asked.contains(&m.id))
        .map(|m| m.id.clone())
        .collect()
}

/// Fills in what a fetched model list of a `protocol` provider leaves out, after fetching
/// models.dev's data when [`to_ask`] finds a model to ask about. A failed fetch leaves the data in
/// use as it was.
pub async fn fill_list(http: reqwest::Client, protocol: Protocol, models: &mut [ModelInfo]) {
    // A System One model only decides (AI-47), and nothing is filled in for it.
    if protocol == Protocol::SystemOne {
        return;
    }
    let ids = to_ask(models, models_dev::knows, &asked());
    if !ids.is_empty() {
        let fetch = tauri::async_runtime::spawn(async move {
            let _one_at_a_time = FETCHING.lock().await;
            // A fetch that ran meanwhile may have answered.
            let ids: Vec<String> = {
                let asked = asked();
                ids.into_iter()
                    .filter(|id| !asked.contains(id) && !models_dev::knows(id))
                    .collect()
            };
            if ids.is_empty() {
                return;
            }
            match models_dev::fetch(&http, &CancellationToken::new()).await {
                Ok(data) => {
                    tracing::info!(models = data.catalog.len(), "fetched models.dev's data");
                    models_dev::install(data.catalog);
                    asked().extend(ids);
                }
                Err(e) => tracing::info!(
                    kind = super::error_kind(&e),
                    status = ?e.status(),
                    "fetching models.dev's data failed"
                ),
            }
        });
        let _ = tokio::time::timeout(LIST_WAIT, fetch).await;
    }
    for model in models {
        model.fill_known(protocol);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dev_is_asked_about_models_neither_the_list_nor_the_data_knows() {
        let known = |id: &str| id.starts_with("claude-");
        let model = |id: &str| ModelInfo {
            id: id.into(),
            name: id.into(),
            ..ModelInfo::default()
        };
        // Anthropic's list gives the limits and levels.
        let complete = ModelInfo {
            context_window: Some(1_000_000),
            max_output_tokens: Some(128_000),
            efforts: Some(Vec::new()),
            ..model("claude-next")
        };
        let mut asked = BTreeSet::new();
        assert!(to_ask(&[model("claude-opus-4-8"), complete], known, &asked).is_empty());
        let list = [
            model("claude-opus-4-8"),
            model("gpt-next"),
            model("llama3.1:8b"),
        ];
        assert_eq!(to_ask(&list, known, &asked), ["gpt-next", "llama3.1:8b"]);
        // A fetch that answered for them: they are not asked about again, while a model the list
        // adds later is. After a failed fetch nothing was added, so the next list asks again.
        asked.extend(to_ask(&list, known, &asked));
        assert!(to_ask(&list, known, &asked).is_empty());
        assert_eq!(
            to_ask(&[model("gpt-next"), model("gpt-newer")], known, &asked),
            ["gpt-newer"]
        );
    }
}
