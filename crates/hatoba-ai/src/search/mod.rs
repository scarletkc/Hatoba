//! The text side of the search tools (spec §13.4, AI-48, AI-49): the glob that `glob` matches and
//! `grep` picks files by, the pattern `grep` hands to the host's `rg`, or to its `grep -E` as
//! translated here, and the results, built from what `find`, `rg` and `grep` printed. Running them
//! on the host is the desktop shell's part, with the scripts in `hatoba_ssh::remote_search`.

mod glob;
mod pattern;
mod results;

#[cfg(test)]
mod tests;

pub use glob::*;
pub use pattern::*;
pub use results::*;
