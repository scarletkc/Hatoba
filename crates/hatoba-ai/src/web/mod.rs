//! The web tools: `web_search` (AI-14), through a search service or the chat provider's own
//! search, and `fetch_url` (AI-15).

mod fetch;
mod native;
mod search;
mod text;

pub use fetch::{FetchResult, MAX_FETCH_CHARS, fetch_url, format_fetch_result};
pub use native::{NativeResults, NativeSearch, format_native_results, native_search, turned_away};
pub use search::{SearchConfig, SearchKind, SearchResult, format_search_results, web_search};
