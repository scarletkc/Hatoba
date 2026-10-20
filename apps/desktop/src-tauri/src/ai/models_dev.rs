//! AI-03: models.dev's data, which fills in what providers' model lists leave out
//! ([`hatoba_ai::models::models_dev`]). The snapshot Hatoba ships knows the models released
//! before the version, and a released model's limits and levels do not change, so models.dev is
//! asked only about a model a fetched list leaves incomplete that the data in use does not know:
//! one newer than the snapshot, or one models.dev does not have. Each is asked about once a run.
//! The form saves what is filled in, and requests use what was saved.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use hatoba_ai::models::{ModelInfo, models_dev};
use hatoba_ai::provider::Protocol;
use tokio_util::sync::CancellationToken;

/// How long a model list waits for models.dev's data; a slower fetch goes on in the background.
const LIST_WAIT: Duration = Duration::from_secs(5);

/// The model ids models.dev was asked about in this run.
static ASKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Whether `models` has a model to ask models.dev about: one the list leaves without a limit or
/// levels, that `known` does not know, and that no earlier list of this run asked about. Those are
/// added to `asked`.
fn to_ask(
    models: &[ModelInfo],
    known: impl Fn(&str) -> bool,
    asked: &mut BTreeSet<String>,
) -> bool {
    let incomplete = |m: &&ModelInfo| {
        m.context_window.is_none() || m.max_output_tokens.is_none() || m.efforts.is_none()
    };
    models
        .iter()
        .filter(incomplete)
        .filter(|m| !known(&m.id))
        .filter(|m| asked.insert(m.id.clone()))
        .count()
        > 0
}

/// Fills in what a fetched model list of a `protocol` provider leaves out, after fetching
/// models.dev's data when [`to_ask`] finds a model to ask about. A failed fetch leaves the data in
/// use as it was.
pub async fn fill_list(http: reqwest::Client, protocol: Protocol, models: &mut [ModelInfo]) {
    // A System One model only decides (AI-47), and nothing is filled in for it.
    if protocol == Protocol::SystemOne {
        return;
    }
    let ask = to_ask(
        models,
        models_dev::knows,
        &mut ASKED.lock().unwrap_or_else(PoisonError::into_inner),
    );
    if ask {
        let fetch = tauri::async_runtime::spawn(async move {
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
    fn models_dev_is_asked_once_a_run_about_models_neither_the_list_nor_the_data_knows() {
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
        assert!(!to_ask(
            &[model("claude-opus-4-8"), complete],
            known,
            &mut asked
        ));
        // A model nothing knows is asked about once, together with the others of its list.
        assert!(to_ask(
            &[
                model("claude-opus-4-8"),
                model("gpt-next"),
                model("llama3.1:8b")
            ],
            known,
            &mut asked
        ));
        assert!(!to_ask(
            &[model("gpt-next"), model("llama3.1:8b")],
            known,
            &mut asked
        ));
        // A model the list adds later is.
        assert!(to_ask(
            &[model("gpt-next"), model("gpt-newer")],
            known,
            &mut asked
        ));
    }
}
