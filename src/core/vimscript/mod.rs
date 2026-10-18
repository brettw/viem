//! Legacy Vim script (the language of `.vimrc` files in Vim 8 and Vim 9's
//! legacy dialect). Viem uses one engine for every script it loads:
//! `startup.viem`, `:source`, and commands typed at the `:` prompt.
//!
//! This module is independent of the command interpreter; the editor plugs
//! in through `ScriptHost`. Vim9 script is not supported.
pub mod autocmd;
mod builtins;
pub(crate) mod commands;
pub mod error;
pub mod exec;
pub mod excmds;
pub(crate) mod expr;
mod function_cmd;
pub mod keys;
mod let_cmd;
mod printf;
pub(crate) mod regex;
pub mod source;
pub mod state;
mod user_commands;
pub mod value;
pub mod vvars;

pub use error::{EvalResult, Exception, Fail, VimError};
pub use exec::{HostCommand, Message, Modifiers, OptionScope, RunState, Runtime, ScriptError, ScriptHost};
pub use state::ScriptState;
pub use value::{Dict, Funcref, List, VStr, Value};

#[cfg(test)]
mod tests;
