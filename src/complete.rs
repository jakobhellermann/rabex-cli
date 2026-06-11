//! Dynamic shell completion helpers.
//!
//! `clap_complete` doesn't hand parsed arguments to a completer, so — like jj
//! does — we re-parse `std::env::args_os()` ourselves to recover the game
//! context and selected file the user already typed, then offer context-aware
//! candidates.

use std::path::PathBuf;

use anyhow::Result;
use clap::{ArgMatches, CommandFactory as _};
use clap_complete::CompletionCandidate;
use rabex_env::Environment;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::tpk::TpkTypeTreeBlob;
use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::resolver::{EnvResolver as _, GameFiles};

use crate::cli::{Cli, Context};
use crate::{ctx, qualify};

/// The concrete handle type the completion helpers operate on.
type Handle<'a> = SerializedFileHandle<'a, GameFiles, TypeTreeCache<TpkTypeTreeBlob>>;

/// Re-parse the in-progress command line into `ArgMatches`.
///
/// Errors are ignored (`ignore_errors`) because the line is mid-edit; we read
/// fields out per-field rather than via `from_arg_matches`, which would also
/// try to parse the (possibly empty/partial) `obj` path id into an `i64`.
fn current_matches() -> Result<ArgMatches> {
    // The clap_complete prelude is `<bin> -- <bin> <actual args...>`, so skip 2.
    let args = std::env::args_os().skip(2);
    Ok(Cli::command()
        .disable_version_flag(true)
        .disable_help_flag(true)
        .ignore_errors(true)
        .try_get_matches_from(args)?)
}

/// The game context flags, read from wherever they were typed (they are global).
fn game_args(matches: &ArgMatches) -> Context {
    Context {
        steam_game: matches.get_one::<String>("steam_game").cloned(),
        game_dir: matches.get_one::<PathBuf>("game_dir").cloned(),
    }
}

/// The game `Environment` for the context currently typed, if any. Completion
/// of game-relative paths only makes sense with a game to enumerate; without
/// one the shell falls back to plain path completion.
fn current_game_env() -> Result<Option<Environment>> {
    ctx::game_env(&game_args(&current_matches()?))
}

fn paths_to_candidates(paths: Vec<PathBuf>) -> Vec<CompletionCandidate> {
    paths
        .into_iter()
        .map(|p| CompletionCandidate::new(p.to_string_lossy().into_owned()))
        .collect()
}

/// Resolve the serialized file selected on the command line and hand its handle
/// to `f`. Handles `scene <name>`, `file <path>` and `bundle <path> file <cab>`.
/// Returns no candidates when no such target is present.
fn with_target_handle(
    f: impl FnOnce(&Handle<'_>) -> Result<Vec<CompletionCandidate>>,
) -> Result<Vec<CompletionCandidate>> {
    let matches = current_matches()?;
    let game = game_args(&matches);

    match matches.subcommand() {
        Some(("file", m)) => {
            let Some(path) = m.get_one::<PathBuf>("path") else {
                return Ok(Vec::new());
            };
            let (env, relative) = ctx::open_file(&game, path)?;
            let handle = env.load_serialized(&relative)?;
            f(&handle)
        }
        Some(("scene", m)) => {
            let Some(name) = m.get_one::<String>("name") else {
                return Ok(Vec::new());
            };
            let env = ctx::require_game_env(&game)?;
            let handle = ctx::open_scene(&env, name)?;
            f(&handle)
        }
        Some(("bundle", m)) => {
            let Some(path) = m.get_one::<PathBuf>("path") else {
                return Ok(Vec::new());
            };
            let Some(("file", fm)) = m.subcommand() else {
                return Ok(Vec::new());
            };
            let Some(cab) = fm.get_one::<String>("cab") else {
                return Ok(Vec::new());
            };
            let (env, bundle) = ctx::open_bundle(&game, path)?;
            let handle = ctx::bundle_serialized(&env, &bundle, Some(cab))?;
            f(&handle)
        }
        _ => Ok(Vec::new()),
    }
}

/// Object references of the selected file (for `object <ref>`): every path id
/// (labelled with class) and every component path. The shell filters by prefix.
pub fn object_refs() -> Result<Vec<CompletionCandidate>> {
    with_target_handle(|handle| {
        let mut candidates: Vec<CompletionCandidate> = handle
            .objects::<()>()
            .map(|obj| {
                CompletionCandidate::new(obj.path_id().to_string())
                    .help(Some(format!("{:?}", obj.class_id()).into()))
            })
            .collect();
        candidates.extend(
            qualify::all_paths(handle)
                .into_iter()
                .map(|path| CompletionCandidate::new(path.to_string())),
        );
        Ok(candidates)
    })
}

/// GameObject paths of the selected file (for `tree <path>`): the component
/// paths without a `@component` selector.
pub fn gameobject_paths() -> Result<Vec<CompletionCandidate>> {
    with_target_handle(|handle| {
        Ok(qualify::all_paths(handle)
            .into_iter()
            .filter(|path| path.component.is_none())
            .map(|path| CompletionCandidate::new(path.to_string()))
            .collect())
    })
}

/// Candidates for a `file <path>`: the game's serialized files (game-relative).
pub fn game_files() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(paths_to_candidates(env.game_files.serialized_files()?))
}

/// Candidates for `bundle <path> file <CAB>`: the serialized files inside the
/// bundle named on the command line.
pub fn bundle_cabs() -> Result<Vec<CompletionCandidate>> {
    use rabex_env::rabex::files::unityfile::FileEntry;

    let matches = current_matches()?;
    let game = game_args(&matches);
    let Some(("bundle", bundle_match)) = matches.subcommand() else {
        return Ok(Vec::new());
    };
    let Some(path) = bundle_match.get_one::<PathBuf>("path") else {
        return Ok(Vec::new());
    };

    let (_env, bundle) = ctx::open_bundle(&game, path)?;
    Ok(bundle
        .files()
        .iter()
        .filter(|entry| entry.flags & FileEntry::FLAG_SERIALIZEDFILE != 0)
        .map(|entry| CompletionCandidate::new(entry.path.clone()))
        .collect())
}

/// Candidates for a `bundle <path>`: the game's addressables bundles (relative
/// to the addressables build folder, the form the command expects with a game).
pub fn bundle_files() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(paths_to_candidates(env.addressables_bundles()?))
}

/// Candidates for `addressable info <KEY>`: every catalog key, with the distinct
/// asset types it resolves to as help text (e.g. `AtmosCue, MusicCue`).
pub fn addressable_keys() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(ctx::addressable_keys(&env)?
        .into_iter()
        .map(|(key, types)| {
            let help = types.into_iter().collect::<Vec<_>>().join(", ");
            CompletionCandidate::new(key).help(Some(help.into()))
        })
        .collect())
}

/// Candidates for a `scene <name>`: built-in + addressables scene names, with
/// their source (`levelN` / bundle) as help text.
pub fn scene_names() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(ctx::scenes(&env)?
        .into_iter()
        .map(|scene| CompletionCandidate::new(scene.name).help(Some(scene.source.label().into())))
        .collect())
}

/// Candidates for `--steam-game`: installed steam games that look like unity
/// games, with their app id as help text.
///
/// Note: completing *in the middle* of a quoted spaced name doesn't work in
/// fish. `commandline --current-token` hands clap the token *with* its opening
/// quote (e.g. `'Hollow Knigh`), and clap prefix-matches that literal against
/// the candidates — which start with no quote — so nothing matches. There's no
/// fish flag to get the token unquoted (fish-shell#10875). Completing at the
/// end of the token works fine. A code fix would mean switching to
/// `ArgValueCompleter` and stripping quotes from the token before filtering
/// ourselves.
pub fn steam_games() -> Vec<CompletionCandidate> {
    fn inner() -> Result<Vec<CompletionCandidate>> {
        let steam = steamlocate::SteamDir::locate()?;
        let mut candidates = Vec::new();

        for library in steam.libraries()?.filter_map(Result::ok) {
            for app in library.apps().filter_map(Result::ok) {
                let app_dir = library.resolve_app_dir(&app);
                if GameFiles::probe_dir(&app_dir).is_err() {
                    continue;
                }
                let name = app.name.clone().unwrap_or_else(|| app.install_dir.clone());
                candidates
                    .push(CompletionCandidate::new(name).help(Some(app.app_id.to_string().into())));
            }
        }
        Ok(candidates)
    }

    inner().unwrap_or_default()
}
