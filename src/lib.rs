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

use crate::cli::{AddressableVerb, AddressablesVerb, Command, GameVerb};
use crate::commands::file::FileLocation;

/// Run a parsed CLI. The binary's `main` is a thin wrapper around this.
pub fn run(cli: crate::cli::Cli) -> Result<()> {
    let game = &cli.game;

    match cli.command {
        // Game summary.
        Command::Game(args) => match args.verb.unwrap_or(GameVerb::Info) {
            GameVerb::Info => commands::game::info(&ctx::require_game_env(game)?),
        },

        // Collections (plural). Bare and `list` both list.
        Command::Scenes(_) => commands::game::scenes(&ctx::require_game_env(game)?),
        Command::Files(_) => commands::game::ls(&ctx::require_game_env(game)?),
        Command::Bundles(_) => commands::bundle::list_all(&ctx::require_game_env(game)?),
        Command::Addressables(args) => {
            let env = ctx::require_game_env(game)?;
            match args.verb.unwrap_or(AddressablesVerb::List) {
                AddressablesVerb::List => commands::game::addressable_ls(&env),
                AddressablesVerb::Stats => commands::game::addressable_stats(&env),
            }
        }

        // Items (singular): select then run a verb.
        Command::Scene(args) => {
            let env = ctx::require_game_env(game)?;
            let (handle, location) = ctx::open_scene(&env, &args.name)?;
            commands::file::run_verb(location, &handle, args.verb)
        }
        Command::File(args) => {
            let (env, relative) = ctx::open_file(game, &args.path)?;
            let handle = env.load_serialized(&relative)?;
            commands::file::run_verb(
                FileLocation::File(args.path.to_str().unwrap().to_owned()),
                &handle,
                args.verb,
            )
        }
        Command::Bundle(args) => commands::bundle::run(game, args),
        Command::Addressable(args) => {
            let env = ctx::require_game_env(game)?;
            let dependencies = match args.verb {
                Some(AddressableVerb::Info(info)) => info.dependencies,
                None => false,
            };
            commands::game::addressable_info(&env, &args.key, dependencies)
        }
    }
}
