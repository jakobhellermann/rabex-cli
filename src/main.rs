use std::io;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;

use rabex_cli::cli::Cli;

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    match rabex_cli::run(cli) {
        Err(e) if is_broken_pipe(&e) => Ok(()),
        result => result,
    }
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    })
}
