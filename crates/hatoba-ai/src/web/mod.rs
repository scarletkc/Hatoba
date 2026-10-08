//! The web tools: `web_search` (AI-14) and `fetch_url` (AI-15).

mod fetch;
mod search;
mod text;

pub use fetch::{FetchResult, MAX_FETCH_CHARS, fetch_url, format_fetch_result};
pub use search::{SearchConfig, SearchKind, SearchResult, format_search_results, web_search};
