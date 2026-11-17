//! Built-in tools (spec §13.4): names, definitions, the system prompt, argument structs and
//! result helpers.
//!
//! [`builtin_tools`] and [`system_prompt`] are deterministic (the same input gives byte-identical
//! output), because prompt caching and the binding of signed thinking blocks depend on an
//! unchanged prefix.

mod args;
mod defs;
mod names;
mod notes;
mod prompt;
mod results;

pub use args::*;
pub use defs::*;
pub use names::*;
pub use notes::*;
pub use prompt::*;
pub use results::*;
