//! Refreshes the snapshot of models.dev's data that Hatoba ships (AI-03),
//! `src/models/models-dev.json`. The release's version bump runs it:
//!
//! ```sh
//! cargo run -p hatoba-ai --example models_dev_snapshot
//! ```

use std::path::Path;

use hatoba_ai::models::models_dev;
use tokio_util::sync::CancellationToken;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = hatoba_ai::provider::http_client();
    let data = runtime.block_on(models_dev::fetch(&http, &CancellationToken::new()))?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/models/models-dev.json");
    std::fs::write(&path, format!("{}\n", data.json))?;
    println!("Wrote {} models to {}", data.catalog.len(), path.display());
    Ok(())
}
