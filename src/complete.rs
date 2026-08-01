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
use crate::commands::file::FileLocation;
use crate::component_path::ObjectRef;
use crate::resolve::resolve_object_ref;
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
/// (and its external name, for enrichment) to `f`. Handles `scene <name>`,
/// `file <path>`, `bundle <path> file <cab>` and `addressable <key> file`.
/// Returns no candidates when no such target is present.
fn with_target_handle(
    f: impl FnOnce(&Handle<'_>, &str) -> Result<Vec<CompletionCandidate>>,
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
            f(&handle, path.to_string_lossy().as_ref())
        }
        Some(("scene", m)) => {
            let Some(name) = m.get_one::<String>("name") else {
                return Ok(Vec::new());
            };
            let env = ctx::require_game_env(&game)?;
            let (handle, location) = ctx::open_scene(&env, name)?;
            f(&handle, &location.external_name())
        }
        Some(("bundle", m)) => {
            let Some(path) = m.get_one::<PathBuf>("path") else {
                return Ok(Vec::new());
            };
            let Some(("file", fm)) = m.subcommand() else {
                return Ok(Vec::new());
            };
            let (env, bundle) = ctx::open_bundle(&game, path)?;
            // `cab` is optional; without it the verb operates on the bundle's
            // main serialized file, so complete against that one.
            let cab = fm.get_one::<String>("cab").map(String::as_str);
            let handle = ctx::bundle_serialized(&env, &bundle, cab)?;
            let cab_name = match cab {
                Some(cab) => cab.to_owned(),
                None => bundle
                    .main_serializedfile()
                    .map(|f| f.path.clone())
                    .unwrap_or_default(),
            };
            f(
                &handle,
                &FileLocation::Bundle { cab: cab_name }.external_name(),
            )
        }
        Some(("addressable", m)) => {
            let Some(key) = m.get_one::<String>("key") else {
                return Ok(Vec::new());
            };
            let env = ctx::require_game_env(&game)?;
            let (handle, location, _asset) = ctx::open_addressable(&env, key)?;
            f(&handle, &location.external_name())
        }
        _ => Ok(Vec::new()),
    }
}

/// The object reference the command line selected (`object <REF> …`), wherever
/// the `object` verb sits under the file/scene/bundle/addressable subcommand.
fn selected_object_ref(matches: &ArgMatches) -> Option<ObjectRef> {
    let mut m = matches;
    while let Some((name, sub)) = m.subcommand() {
        if name == "object" {
            return sub.get_one::<ObjectRef>("reference").cloned();
        }
        m = sub;
    }
    None
}

/// Object references of the selected file (for `object <ref>`): every path id
/// (labelled with class), every object's `m_Name`, every class name (for
/// singletons like `TagManager`), and every component path. The shell filters by
/// prefix.
pub fn object_refs() -> Result<Vec<CompletionCandidate>> {
    with_target_handle(|handle, _name| {
        let mut candidates: Vec<CompletionCandidate> = handle
            .objects::<()>()
            .map(|obj| {
                CompletionCandidate::new(obj.path_id().to_string())
                    .help(Some(format!("{:?}", obj.class_id()).into()))
            })
            .collect();

        // Component paths double as names for GameObjects; don't also offer the
        // bare `m_Name` for those (it would resolve to the same object).
        let paths = qualify::all_paths(handle);
        let mut seen: std::collections::HashSet<String> =
            paths.iter().map(|p| p.to_string()).collect();

        for obj in handle.objects::<()>() {
            let class = format!("{:?}", obj.class_id());
            // Selectable by class name (deduped) — useful for class-typed singletons.
            if seen.insert(class.clone()) {
                candidates.push(CompletionCandidate::new(class.clone()).help(Some("class".into())));
            }
            // Selectable by `m_Name` (e.g. a `MonoScript`'s class name).
            let name = handle
                .object_at::<serde_json::Value>(obj.path_id())
                .and_then(|o| o.read())
                .ok()
                .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()).map(str::to_owned))
                .filter(|name| !name.is_empty());
            if let Some(name) = name
                && seen.insert(name.clone())
            {
                candidates.push(CompletionCandidate::new(name).help(Some(class.into())));
            }
        }

        candidates.extend(
            paths
                .into_iter()
                .map(|path| CompletionCandidate::new(path.to_string())),
        );
        Ok(candidates)
    })
}

/// Field paths into the enriched object for `object <ref> cat --jq <FILTER>`.
///
/// Only a bare leading dot-path token is completed (`.m_Foo`, `.a.b.`,
/// `.a[0].b`), one level at a time: `.a.` offers `.a.b`, `.a.c`. The already-typed
/// path prefix is handed to jaq, so it — not us — does the traversal: array
/// indices (`.a[0].`) and `[]` iteration (`.a[].`, whose element keys are unioned)
/// both descend. Anything with a space, `|`, `(` or `{` is left alone — that's a
/// real expression, not a field path. Completes against the *enriched* object (the
/// value the query runs over), so PPtrs expose `.file` / `.path_id` / `.class_id`
/// and the added `_file` / `_type` keys show up.
///
/// When the partial matches a *single* descendible field, a second candidate
/// ending in the accessor that continues into it (`.key.` for an object, `.key[`
/// for an array) is added too. The two shared-prefix candidates keep the shell
/// from finishing the token with a space, so you can keep drilling in; with
/// several matches the shell already stops at the common prefix, so it's omitted.
pub fn jq_paths(current: &std::ffi::OsStr) -> Result<Vec<CompletionCandidate>> {
    let Some(token) = current.to_str() else {
        return Ok(Vec::new());
    };
    // Stay out of the way of real jq expressions; we only complete a field path.
    // `[` / `]` pass through for indices — jaq evaluates the prefix and simply
    // yields nothing useful for malformed ones.
    if (!token.is_empty() && !token.starts_with('.'))
        || token.contains(|c: char| c.is_whitespace() || "|(){}".contains(c))
    {
        return Ok(Vec::new());
    }

    let reference = selected_object_ref(&current_matches()?);
    let Some(reference) = reference else {
        return Ok(Vec::new());
    };

    with_target_handle(|handle, name| {
        use rabex_jq::jaq_json::Val;
        use rabex_jq::jaq_json::write::{self, Pp};
        use rabex_jq::{Enrich, QueryRunner, enrich};

        let path_id = resolve_object_ref(handle, &reference)?;
        let object = handle.object_at::<Val>(path_id)?;
        let script = object.mono_script()?;
        let mut value = object.read()?;
        // No `SceneIndex` here — building it parses the whole catalog, too slow for
        // a keystroke; the only cost is `_scene` not being offered on scene objects.
        enrich(
            &mut value,
            name,
            handle,
            Enrich {
                scenes: None,
                script: script.as_ref(),
            },
        )?;

        // `base` = the token up to and including its last `.`; the tail after it is
        // the partial key the shell filters on. The prefix jaq evaluates is `base`
        // without that trailing `.` (`.` at the root), so jaq handles any `[n]` /
        // `[]` accessors along the path.
        let base = if token.is_empty() {
            "."
        } else {
            let last = token.rfind('.').expect("non-empty token starts with '.'");
            &token[..=last]
        };
        let prefix = base
            .strip_suffix('.')
            .filter(|s| !s.is_empty())
            .unwrap_or(".");
        let Ok(runner) = QueryRunner::new(prefix) else {
            return Ok(Vec::new());
        };

        // The fields at the prefix that the shell would actually show (matching the
        // partial), unioned across `[]` iteration results and deduped by accessor.
        // Each carries the accessor (`.` / `[`) that would descend into it, if any.
        let mut fields: Vec<(String, Option<char>, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for result in runner.exec(handle.env, value)? {
            let mut buf = Vec::new();
            if write::write(&mut buf, &Pp::default(), 0, &result).is_err() {
                continue;
            }
            let Ok(serde_json::Value::Object(map)) =
                serde_json::from_slice::<serde_json::Value>(&buf)
            else {
                continue;
            };
            for (key, child) in &map {
                // Bare jq identifiers stay `.key`; anything else must be quoted
                // (`."data[0]"`), or jq reads the brackets as an index accessor.
                let field = if is_jq_ident(key) {
                    key.clone()
                } else {
                    serde_json::to_string(key)?
                };
                let leaf = format!("{base}{field}");
                if !leaf.starts_with(token) || !seen.insert(leaf.clone()) {
                    continue;
                }
                let descend = match child {
                    serde_json::Value::Object(_) => Some('.'),
                    serde_json::Value::Array(a) if !a.is_empty() => Some('['),
                    _ => None,
                };
                fields.push((leaf, descend, value_hint(child)));
            }
        }

        // When a single field matches, also offer the accessor that descends into
        // it (`.foo.` / `.foo[`): the two shared-prefix candidates make the shell
        // fill the common prefix instead of ending the token with a space, so you
        // can keep drilling. With several matches the shell already stops at the
        // common prefix, so the extra candidate would just be noise.
        let sole = fields.len() == 1;
        let mut candidates = Vec::new();
        for (leaf, descend, hint) in fields {
            let descend_form = descend.filter(|_| sole).map(|sep| {
                CompletionCandidate::new(format!("{leaf}{sep}")).help(Some(hint.clone().into()))
            });
            candidates.push(CompletionCandidate::new(leaf).help(Some(hint.into())));
            candidates.extend(descend_form);
        }
        Ok(candidates)
    })
}

/// Whether `key` can follow a jq `.` bare (`.m_Name`) rather than needing quotes
/// (`."data[0]"`): a leading letter/`_`, then letters/digits/`_`.
fn is_jq_ident(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A short one-line summary of a JSON value, shown as completion help: the scalar
/// itself, or a `{n}` / `[n]` size for containers.
fn value_hint(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let preview: String = s.chars().take(40).collect();
            let ellipsis = if preview.len() < s.len() { "…" } else { "" };
            format!("\"{preview}{ellipsis}\"")
        }
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(o) => format!("{{{}}}", o.len()),
    }
}

/// Object class names of the selected file (for `objects --type`): the distinct
/// `ClassId` names, plus MonoBehaviour script class names (which `--type` also
/// matches).
pub fn object_types() -> Result<Vec<CompletionCandidate>> {
    use rabex_env::rabex::objects::{ClassId, PPtr};

    with_target_handle(|handle, _name| {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for obj in handle.objects::<()>() {
            let class_id = obj.class_id();
            let class = format!("{class_id:?}");
            if seen.insert(class.clone()) {
                out.push(CompletionCandidate::new(class.clone()));
            }
            if class_id == ClassId::MonoBehaviour
                && let Ok(script) =
                    crate::resolve::component_label(handle, PPtr::local(obj.path_id()))
                && script != class
                && seen.insert(script.clone())
            {
                out.push(CompletionCandidate::new(script).help(Some("script".into())));
            }
        }
        Ok(out)
    })
}

/// Component/script type names of the selected file (for `find <TYPE>`): the
/// distinct `@component` labels across the hierarchy.
pub fn component_types() -> Result<Vec<CompletionCandidate>> {
    with_target_handle(|handle, _name| {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for path in qualify::all_paths(handle) {
            if let Some(component) = path.component
                && seen.insert(component.name.clone())
            {
                out.push(CompletionCandidate::new(component.name));
            }
        }
        Ok(out)
    })
}

/// GameObject paths of the selected file (for `tree <path>`): the component
/// paths without a `@component` selector.
pub fn gameobject_paths() -> Result<Vec<CompletionCandidate>> {
    with_target_handle(|handle, _name| {
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

/// Candidates for `references --include/--exclude`: the file paths that can show
/// up as a referrer — the game's serialized files plus the addressables bundles.
/// Cheap: just enumerates paths, no per-file parsing. The flags match any
/// substring, so a full path is one valid choice among many.
pub fn referrer_files() -> Result<Vec<CompletionCandidate>> {
    let Some(env) = current_game_env()? else {
        return Ok(Vec::new());
    };
    let mut paths = env.game_files.serialized_files()?;
    paths.extend(env.addressables_bundles()?);
    Ok(paths_to_candidates(paths))
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
    // Mirror the default `addressables` listing: omit internal AssetBundle keys.
    Ok(ctx::addressable_keys(&env, false)?
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

#[cfg(test)]
mod tests {
    use super::{is_jq_ident, value_hint};
    use serde_json::json;

    #[test]
    fn jq_ident_needs_a_leading_letter_or_underscore_then_word_chars() {
        assert!(is_jq_ident("m_Name"));
        assert!(is_jq_ident("_file"));
        assert!(is_jq_ident("a1_b"));

        assert!(!is_jq_ident(""));
        assert!(!is_jq_ident("1abc"));
        assert!(!is_jq_ident("data[0]"));
        assert!(!is_jq_ident("with space"));
        assert!(!is_jq_ident("a-b"));
    }

    #[test]
    fn value_hint_summarises_each_json_kind() {
        assert_eq!(value_hint(&json!(null)), "null");
        assert_eq!(value_hint(&json!(true)), "true");
        assert_eq!(value_hint(&json!(3)), "3");
        assert_eq!(value_hint(&json!("hi")), "\"hi\"");
        assert_eq!(value_hint(&json!([1, 2, 3])), "[3]");
        assert_eq!(value_hint(&json!({ "a": 1, "b": 2 })), "{2}");
    }

    #[test]
    fn value_hint_truncates_long_strings_to_40_chars() {
        let long = "a".repeat(45);
        assert_eq!(value_hint(&json!(long)), format!("\"{}…\"", "a".repeat(40)));
    }
}
