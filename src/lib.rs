pub mod cli;
pub mod complete;
pub mod component_path;
pub mod ctx;
pub mod locate;
pub mod qualify;
pub mod commands {
    pub mod bundle;
    pub mod file;
    pub mod game;
}

use anyhow::Result;

use crate::cli::{Cli, Command};

/// Run a parsed CLI. The binary's `main` is a thin wrapper around this.
pub fn run(cli: Cli) -> Result<()> {
    let game = &cli.game;

    match cli.command {
        Command::Info => commands::game::info(&ctx::require_game_env(game)?),
        Command::Ls => commands::game::ls(&ctx::require_game_env(game)?),
        Command::Scenes => commands::game::scenes(&ctx::require_game_env(game)?),
        Command::Addressables(args) => {
            let env = ctx::require_game_env(game)?;
            match args.command {
                cli::AddressablesCmd::Stats => commands::game::addressable_stats(&env),
                cli::AddressablesCmd::Ls => commands::game::addressable_ls(&env),
                cli::AddressablesCmd::Info(info) => {
                    commands::game::addressable_info(&env, &info.key, info.dependencies)
                }
            }
        }
        Command::Bundle(args) => commands::bundle::run(game, args),
        Command::File(args) => {
            let (env, relative) = ctx::open_file(game, &args.path)?;
            let handle = env.load_serialized(&relative)?;
            commands::file::run(&handle, args.verb)
        }
        Command::Scene(args) => {
            let env = ctx::require_game_env(game)?;
            let handle = ctx::open_scene(&env, &args.name)?;
            commands::file::run(&handle, args.verb)
        }
    }
}
