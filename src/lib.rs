pub mod cli;
pub mod complete;
pub mod component_path;
pub mod ctx;
pub mod locate;
pub mod output;
pub mod qualify;
pub mod commands {
    pub mod bundle;
    pub mod file;
    pub mod game;
}

use anyhow::Result;

use crate::cli::{
    AddressableInfoArgs, AddressableVerb, AddressablesVerb, Command, FileVerb, GameVerb,
    ObjectArgs, ObjectVerb,
};
use crate::commands::file::FileLocation;
use crate::component_path::ObjectRef;

/// Run a parsed CLI. The binary's `main` is a thin wrapper around this.
pub fn run(cli: crate::cli::Cli) -> Result<()> {
    let game = &cli.game;
    let format = cli.output.format;

    match cli.command {
        // Game summary.
        Command::Game(args) => match args.verb.unwrap_or(GameVerb::Info) {
            GameVerb::Info => commands::game::info(&ctx::require_game_env(game)?, format),
            GameVerb::ScriptLocations(args) => commands::game::script_locations(
                &ctx::require_game_env(game)?,
                args.filter.as_deref(),
                format,
            ),
        },

        // Collections (plural). Bare and `list` both list.
        Command::Scenes(_) => commands::game::scenes(&ctx::require_game_env(game)?, format),
        Command::Files(_) => commands::game::ls(&ctx::require_game_env(game)?, format),
        Command::Bundles(_) => commands::bundle::list_all(&ctx::require_game_env(game)?, format),
        Command::Addressables(args) => {
            let env = ctx::require_game_env(game)?;
            match args.verb.unwrap_or(AddressablesVerb::List) {
                AddressablesVerb::List => {
                    commands::game::addressable_ls(&env, args.include_asset_bundles, format)
                }
                AddressablesVerb::Stats => commands::game::addressable_stats(&env, format),
            }
        }

        // Items (singular): select then run a verb.
        Command::Scene(args) => {
            let env = ctx::require_game_env(game)?;
            let (handle, location) = ctx::open_scene(&env, &args.name)?;
            commands::file::run_verb(location, &handle, args.verb, format)
        }
        Command::File(args) => {
            let (env, relative) = ctx::open_file(game, &args.path)?;
            let handle = env.load_serialized(&relative)?;
            commands::file::run_verb(
                FileLocation::File(args.path.to_str().unwrap().to_owned()),
                &handle,
                args.verb,
                format,
            )
        }
        Command::Bundle(args) => commands::bundle::run(game, args, format),
        Command::Addressable(args) => {
            let env = ctx::require_game_env(game)?;
            let verb = args
                .verb
                .unwrap_or(AddressableVerb::Info(AddressableInfoArgs {
                    dependencies: false,
                }));
            match verb {
                AddressableVerb::Info(info) => {
                    commands::game::addressable_info(&env, &args.key, info.dependencies, format)
                }
                // `cat` is sugar for descending into the bundle's main CAB and
                // dumping the container's main asset by path id.
                AddressableVerb::Cat => {
                    let (handle, location, asset) = ctx::open_addressable(&env, &args.key)?;
                    let verb = FileVerb::Object(ObjectArgs {
                        reference: ObjectRef::PathId(asset),
                        verb: Some(ObjectVerb::Cat),
                    });
                    commands::file::run_verb(location, &handle, Some(verb), format)
                }
                AddressableVerb::File(file) => {
                    let (handle, location, _asset) = ctx::open_addressable(&env, &args.key)?;
                    commands::file::run_verb(location, &handle, file.verb, format)
                }
            }
        }
    }
}
