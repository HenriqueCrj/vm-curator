// Library target for GUI and other external consumers.
// The `ui` and `app` modules are intentionally excluded:
// `ui` contains ratatui/crossterm TUI rendering code, and `app`
// references `ui` types, so neither can be part of the public lib API.
pub mod commands;
pub mod config;
pub mod fs;
pub mod hardware;
pub mod metadata;
pub mod vm;
pub mod wizard_types;
