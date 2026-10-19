//! The LaserFoundry engine: a [`Session`] owns the document, selection and undo history, and every
//! user-visible action is a [`CommandSpec`] in the registry. Menus, the UI, the CLI and (later)
//! MCP all go through [`Session::execute`], so anything a person can do an agent can do.

mod cmd;
mod error;
pub mod fontlib;
mod guard;
mod session;

pub use cmd::{CommandSpec, commands, find_command};
pub use error::EngineError;
pub use session::{HISTORY_LIMIT, Session};

#[cfg(test)]
mod tests;
