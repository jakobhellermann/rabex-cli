pub mod cli;
pub mod complete;
pub mod ctx;
pub mod target;
pub mod commands {
    pub mod info;
    pub mod ls;
    pub mod obj;
}

use anyhow::Result;

use crate::cli::Command;
use crate::ctx::Ctx;

/// Run a parsed command. The binary's `main` is a thin wrapper around this.
pub fn run(command: Command) -> Result<()> {
    let ctx = Ctx::new(command.path())?;

    match command {
        Command::Info(_) => commands::info::run(&ctx),
        Command::Ls(args) => commands::ls::run(&ctx, args),
        Command::Obj(args) => commands::obj::run(&ctx, args),
    }
}
