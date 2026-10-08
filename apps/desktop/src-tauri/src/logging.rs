//! Daily-rotated local logs kept for 7 days (spec §11). Never log secrets or terminal content (SEC-04).

use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Targets that never reach the log, whatever `HATOBA_LOG` says. SEC-04: the MCP SDK logs
/// protocol messages (tool arguments and results), server info, session ids and request errors
/// with their URLs, so it is silenced by a filter of its own that `HATOBA_LOG` cannot override.
const SILENCED: [&str; 1] = ["rmcp"];

fn silenced() -> Targets {
    SILENCED.iter().fold(
        Targets::new().with_default(LevelFilter::TRACE),
        |targets, target| targets.with_target(*target, LevelFilter::OFF),
    )
}

pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_env("HATOBA_LOG").unwrap_or_else(|_| {
        EnvFilter::new("info,russh=warn,russh_sftp=warn,tao=warn,wry=warn,reqwest=warn,hyper=warn")
    });
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("hatoba")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir)
        .ok();
    let (writer, guard) = match file {
        Some(appender) => {
            let (w, g) = tracing_appender::non_blocking(appender);
            (Some(w), Some(g))
        }
        None => (None, None),
    };
    let registry = tracing_subscriber::registry().with(filter).with(silenced());
    let stderr = cfg!(debug_assertions).then(|| fmt::layer().with_target(true));
    let file_layer = writer.map(|w| fmt::layer().with_ansi(false).with_writer(w));
    let _ = registry.with(stderr).with(file_layer).try_init();
    guard
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    /// A writer the test reads back.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rmcp_stays_silent_even_when_the_env_filter_asks_for_it() {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("trace,rmcp=trace,rmcp::service=trace"))
            .with(super::silenced())
            .with(
                fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || writer.clone()),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "rmcp::service", "SESSION-SECRET");
            tracing::error!(target: "rmcp", "TOOL-RESULT-SECRET");
            tracing::info!(target: "hatoba_desktop_lib::mcp", "kept");
        });
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("kept"), "{text}");
        assert!(!text.contains("SECRET"), "{text}");
    }
}
