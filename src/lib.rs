pub mod cli;
pub mod complete;
pub mod ctx;
pub mod locate;
pub mod target;
pub mod commands {
    pub mod info;
    pub mod ls;
    pub mod obj;
}

use anyhow::Result;

use crate::cli::Cli;
use crate::ctx::Ctx;

/// Run a parsed CLI. The binary's `main` is a thin wrapper around this.
pub fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx::new(&cli.target)?;

    match cli.command {
        cli::Command::Info => commands::info::run(&ctx),
        cli::Command::Ls(args) => commands::ls::run(&ctx, args),
        cli::Command::Obj(args) => commands::obj::run(&ctx, args),
    }
}
