pub mod cli;
pub mod complete;
pub mod component_path;
pub mod ctx;
pub mod json;
pub mod locate;
pub mod output;
pub mod qualify;
pub mod resolve;
pub mod commands {
    pub mod addressable;
    pub mod addressables;
    pub mod assembly;
    pub mod bundle;
    pub mod file;
    pub mod files;
    pub mod game;
    pub mod scenes;
    pub mod tpk;
}

use anyhow::{Context as _, Result};
use rabex_env::unity::types::MonoScript;

use crate::cli::{
    AddressablesVerb, AssemblyVerb, ClassVerb, ClassesArgs, Command, FileVerb, GameVerb,
    ObjectArgs, ObjectVerb, ScriptVerb,
};
use crate::commands::assembly;
use crate::commands::file::FileLocation;
use crate::component_path::ObjectRef;
use crate::output::emit;

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
        Command::Addressables(args) => match args.verb.unwrap_or(AddressablesVerb::List) {
            AddressablesVerb::List => commands::addressables::addressable_ls(
                &ctx::require_game_env(game)?,
                args.include_asset_bundles,
                format,
            ),
            AddressablesVerb::Stats => {
                commands::addressables::addressable_stats(&ctx::require_game_env(game)?, format)
            }
            AddressablesVerb::Catalog(args) => match args.file {
                Some(file) => commands::addressables::catalog_file(&file, format),
                None => {
                    commands::addressables::game_catalogs(&ctx::require_game_env(game)?, format)
                }
            },
        },

        // Items (singular): select then run a verb.
        Command::Scene(args) => {
            let env = ctx::require_game_env(game)?;
            let (handle, location) = ctx::open_scene(&env, &args.name)?;
            commands::file::run_verb(location, &handle, args.verb, format)
        }
        Command::File(args) => {
            let (env, relative) = ctx::open_file(game, &args.path)?;
            let handle = ctx::load_file(&env, &relative, &args.path)?;
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
            let object_verb = match args.verb.unwrap_or(ScriptVerb::Info) {
                ScriptVerb::Info => ObjectVerb::Info,
                ScriptVerb::Cat(args) => ObjectVerb::Cat(args),
                ScriptVerb::References(args) => ObjectVerb::References(args),
                ScriptVerb::Decompile => {
                    let script = handle
                        .objects_of::<MonoScript>()
                        .find(|script| script.path_id() == path_id)
                        .with_context(|| format!("no MonoScript #{path_id}"))?
                        .read()?;
                    let path = assembly::resolve_assembly(&env, &script.m_AssemblyName)?;
                    let class = assembly::resolve_class(&path, &script.full_name())?;
                    let decompiled =
                        assembly::decompile(&assembly::managed_dir(&env)?, &path, &class)?;
                    return emit(&decompiled, format, &mut std::io::stdout().lock());
                }
            };
            let verb = FileVerb::Object(ObjectArgs {
                reference: ObjectRef::PathId(path_id),
                verb: Some(object_verb),
            });
            commands::file::run_verb(location, &handle, Some(verb), format)
        }
        Command::Assemblies(_) => assembly::run_assemblies(&ctx::require_game_env(game)?, format),
        Command::Assembly(args) => {
            let env = ctx::require_game_env(game)?;
            let path = assembly::resolve_assembly(&env, &args.name)?;
            let mut out = std::io::stdout().lock();
            match args
                .verb
                .unwrap_or(AssemblyVerb::Classes(ClassesArgs::default()))
            {
                AssemblyVerb::Classes(args) => emit(
                    &assembly::list_classes(&path, args.filter.as_deref())?,
                    format,
                    &mut out,
                ),
                AssemblyVerb::Class(args) => {
                    let class = assembly::resolve_class(&path, &args.name)?;
                    match args.verb.unwrap_or(ClassVerb::Decompile) {
                        ClassVerb::Decompile => emit(
                            &assembly::decompile(&assembly::managed_dir(&env)?, &path, &class)?,
                            format,
                            &mut out,
                        ),
                    }
                }
            }
        }
        Command::Tpk(args) => commands::tpk::run(args, format),
    }
}
