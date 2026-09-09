pub mod cli;
pub mod complete;
pub mod component_path;
pub mod ctx;
pub mod locate;
pub mod output;
pub mod qualify;
pub mod resolve;
pub mod commands {
    pub mod addressable;
    pub mod addressables;
    pub mod bundle;
    pub mod file;
    pub mod files;
    pub mod game;
    pub mod scenes;
}

use anyhow::Result;

use crate::cli::{AddressablesVerb, Command, FileVerb, GameVerb, ObjectArgs};
use crate::commands::file::FileLocation;
use crate::component_path::ObjectRef;

/// Run a parsed CLI. The binary's `main` is a thin wrapper around this.
pub fn run(cli: crate::cli::Cli) -> Result<()> {
    let game = &cli.game;
    let format = cli.output.format;

    use std::io::IsTerminal as _;
    let color = match cli.output.color {
        crate::cli::ColorChoice::Always => true,
        crate::cli::ColorChoice::Never => false,
        crate::cli::ColorChoice::Auto => {
            std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
        }
    };
    crate::output::set_color(color);

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
        Command::Scenes(_) => commands::scenes::scenes(&ctx::require_game_env(game)?, format),
        Command::Files(_) => commands::files::ls(&ctx::require_game_env(game)?, format),
        Command::Bundles(_) => commands::bundle::list_all(&ctx::require_game_env(game)?, format),
        Command::Addressables(args) => {
            let env = ctx::require_game_env(game)?;
            match args.verb.unwrap_or(AddressablesVerb::List) {
                AddressablesVerb::List => {
                    commands::addressables::addressable_ls(&env, args.include_asset_bundles, format)
                }
                AddressablesVerb::Stats => commands::addressables::addressable_stats(&env, format),
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
            commands::addressable::run(&ctx::require_game_env(game)?, args, format)
        }
        Command::Script(args) => {
            let env = ctx::require_game_env(game)?;
            let (handle, location, path_id) = ctx::locate_script(&env, &args.name)?;
            let verb = FileVerb::Object(ObjectArgs {
                reference: ObjectRef::PathId(path_id),
                verb: args.verb,
            });
            commands::file::run_verb(location, &handle, Some(verb), format)
        }
    }
}
