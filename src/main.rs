mod cli;
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

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    match cli.command {
        Command::Info(args) => commands::info::run(args),
        Command::Ls(args) => commands::ls::run(args),
        Command::Obj(args) => commands::obj::run(args),
    }
}
