#![forbid(unsafe_code)]
pub mod client;
pub mod cmd;
pub mod control;
pub mod format;
pub mod ids;
pub mod layout;
pub mod model;
pub mod modes;
pub mod options;
pub mod server;
/// C reference helpers shared with the other crates' tests (one copy; clippy
/// rejects including the same file as two modules).
#[cfg(test)]
#[path = "../../rmux-util/tests/common/mod.rs"]
pub(crate) mod test_common;
pub mod tsp;
pub mod ui;
