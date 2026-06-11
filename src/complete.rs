//! Dynamic shell completion helpers.
//!
//! `clap_complete` doesn't hand parsed arguments to a completer, so — like jj
//! does — we re-parse `std::env::args_os()` ourselves to recover the target
//! flags the user already typed, then offer context-aware candidates.

use anyhow::Result;
use clap::{CommandFactory as _, FromArgMatches as _};
use clap_complete::CompletionCandidate;
use rabex_env::resolver::{EnvResolver as _, GameFiles};

use crate::cli::{Cli, TargetArgs};
use crate::ctx::Ctx;

/// Recover the target-selection flags of the subcommand being completed.
///
/// Reads them out of `ArgMatches` per-field rather than via
/// `Cli::from_arg_matches`, because the latter also parses the (in-progress)
/// `path_id` into an `i64` — which fails on an empty/partial value and would
/// abort completion exactly when the user is mid-argument.
fn current_target() -> Result<TargetArgs> {
    // The clap_complete prelude is `<bin> -- <bin> <actual args...>`, so skip 2.
    let args = std::env::args_os().skip(2);

    let matches = Cli::command()
        .disable_version_flag(true)
        .disable_help_flag(true)
        .ignore_errors(true)
        .try_get_matches_from(args)?;

    // Target flags live on the top-level command, before the subcommand.
    Ok(TargetArgs::from_arg_matches(&matches)?)
}

/// Candidates for an object path id: every path id in the target file, labelled
/// with its class id. Clap filters these against the typed prefix.
pub fn path_ids() -> Result<Vec<CompletionCandidate>> {
    let ctx = Ctx::new(&current_target()?)?;
    let file = ctx.load()?;

    let candidates = file
        .objects::<()>()
        .map(|obj| {
            CompletionCandidate::new(obj.path_id().to_string())
                .help(Some(format!("{:?}", obj.class_id()).into()))
        })
        .collect();
    Ok(candidates)
}

/// The `Environment` for the game context currently typed, if any.
/// `--file`/`--bundle` completion only makes sense with a game to enumerate;
/// without one the shell falls back to plain path completion.
fn current_game_env() -> Result<Option<rabex_env::Environment>> {
    use rabex_env::rabex::tpk::TpkTypeTreeBlob;
    use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;

    let target = current_target()?;
    let game_dir = match (&target.steam_game, &target.game_dir) {
        (Some(name), None) => crate::locate::locate_steam_game(name)?,
        (None, Some(dir)) => dir.clone(),
        _ => return Ok(None),
    };

    let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
    Ok(Some(rabex_env::Environment::new_in(&game_dir, tpk)?))
}

fn paths_to_candidates(paths: Vec<std::path::PathBuf>) -> Vec<CompletionCandidate> {
    paths
        .into_iter()
        .map(|p| CompletionCandidate::new(p.to_string_lossy().into_owned()))
        .collect()
}

/// Candidates for `--file`: the game's serialized files (game-relative).
pub fn game_files() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(paths_to_candidates(env.game_files.serialized_files()?))
}

/// Candidates for `--bundle`: the game's addressables bundles (relative to the
/// addressables build folder, the form `--bundle` expects with a game context).
pub fn bundle_files() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    Ok(paths_to_candidates(env.addressables_bundles()?))
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
