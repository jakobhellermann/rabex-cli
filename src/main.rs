use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;

use rabex_cli::cli::Cli;

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    rabex_cli::run(cli.command)
}
