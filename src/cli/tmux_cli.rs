#![allow(dead_code)]

pub mod attach;
#[cfg(test)]
mod tests;
pub mod tui_model;
pub mod tui_render;
pub mod tui_run;
pub mod tui_terminal;
/// TMUX control-plane integration
pub mod verbs;

// Re-export public items for external callers (cli.rs)
pub(super) use verbs::launch;
