mod cli;
mod ctx;
mod target;
mod commands {
    pub mod info;
    pub mod ls;
    pub mod obj;
}

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;

use crate::cli::{Cli, Command};
use crate::ctx::Ctx;

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    let ctx = Ctx::new(cli.command.path())?;

    match cli.command {
        Command::Info(_) => commands::info::run(&ctx),
        Command::Ls(args) => commands::ls::run(&ctx, args),
        Command::Obj(args) => commands::obj::run(&ctx, args),
    }
}
