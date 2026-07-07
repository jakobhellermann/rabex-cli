//! The reference-finding file verbs: `references` (files that reference this
//! file) and `object <ref> references` (objects that reference an object, with
//! `--files-with-matches`). Shared by the `file`, `scene` and `bundle <path>
//! file` commands via [`super::file::run_verb`].

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use rabex_env::Environment;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::PPtr;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use serde::Serialize;

use super::FileLocation;
use crate::output::{Render, style};
use crate::resolve::component_class_and_label;

#[derive(Serialize)]
#[serde(transparent)]
pub struct FileReferences(pub Vec<PathBuf>);

impl Render for FileReferences {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        if self.0.is_empty() {
            writeln!(out, "No references found")?;
        }
        for reference in &self.0 {
            writeln!(out, "- {}", reference.display())?;
        }
        Ok(())
    }
}

/// Find every file that references the given file (via its externals list).
pub fn references<R: EnvResolver, P: TypeTreeProvider + Sync>(
    file_location: FileLocation,
    handle: &SerializedFileHandle<'_, R, P>,
) -> Result<FileReferences> {
    let references =
        find_references::external_references(handle.env, &file_location.external_name())?;
    Ok(FileReferences(references))
}

/// One object that references a target object.
#[derive(Serialize)]
pub struct Referrer {
    /// Readable location: the bundle path, or the serialized file path.
    file: String,
    /// Scene name of `file` when it is a built-in `levelN` scene (from
    /// `BuildSettings`); shown as `Menu_Title (level1)`. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    scene: Option<String>,
    path_id: PathId,
    /// Resolved object: its class / script name, plus its `m_Name` or the
    /// GameObject it sits on. Empty when the object could not be deserialized.
    #[serde(skip_serializing_if = "String::is_empty")]
    label: String,
}

/// Header line for a references listing, with the count-appropriate `noun`. When
/// the scan stopped early at `--limit` the true total is unknown, so it flags
/// that; otherwise it is the plain `"{shown} {noun} {target}:"` form (`shown` is
/// then the full total).
fn references_header(
    singular: &str,
    plural: &str,
    target: &str,
    shown: usize,
    truncated: bool,
) -> String {
    let noun = if shown == 1 { singular } else { plural };
    if truncated {
        format!("first {shown} {noun} {target} (--limit):")
    } else {
        format!("{shown} {noun} {target}:")
    }
}

/// Every object that references a target object (local or from another file).
#[derive(Serialize)]
pub struct ObjectReferences {
    /// The target object's `m_Name` (e.g. a `MonoScript`'s class name), else `#<path id>`.
    target: String,
    path_id: PathId,
    /// Whether the scan stopped early at `--limit` (so more referrers may exist).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    referrers: Vec<Referrer>,
}

impl Render for ObjectReferences {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{}",
            style::header(&references_header(
                "reference to",
                "references to",
                &self.target,
                self.referrers.len(),
                self.truncated,
            ))
        )?;
        // Render a `levelN` scene as `Menu_Title (level1)`; plain files unchanged.
        let display = |r: &Referrer| match &r.scene {
            Some(scene) => format!("{scene} ({})", r.file),
            None => r.file.clone(),
        };
        // Pad the file column so the path ids line up. Cap it so a few long
        // addressables bundle paths don't pad every other row to their width —
        // those few overflow ragged instead.
        const FILE_WIDTH_CAP: usize = 40;
        let file_width = self
            .referrers
            .iter()
            .map(|r| display(r).len())
            .max()
            .unwrap_or(0)
            .min(FILE_WIDTH_CAP);
        // Path ids are bimodal: classic serialized files (scenes, `sharedassets`,
        // `levelN`) carry short sequential ids, addressables-bundle objects carry
        // 18-20 digit hash ids — and the two never mix within one file. A single
        // shared width would pad the short ids out to the hash width (a huge gap),
        // so right-align each id within the width of *its own* size class: short
        // ids line up with short ids, hash ids with hash ids.
        const ID_SMALL_MAX: usize = 12;
        let id_len = |r: &Referrer| r.path_id.to_string().len();
        let width_where = |keep: fn(usize) -> bool| {
            self.referrers
                .iter()
                .map(id_len)
                .filter(|len| keep(*len))
                .max()
                .unwrap_or(0)
        };
        let small_width = width_where(|len| len <= ID_SMALL_MAX);
        let hash_width = width_where(|len| len > ID_SMALL_MAX);
        for referrer in &self.referrers {
            let file = style::name(&format!("{:<file_width$}", display(referrer)));
            let id_width = if id_len(referrer) <= ID_SMALL_MAX {
                small_width
            } else {
                hash_width
            };
            let id = style::dim(&format!("{:>id_width$}", referrer.path_id));
            if referrer.label.is_empty() {
                writeln!(out, "- {file}  {id}")?;
            } else {
                writeln!(out, "- {file}  {id}  {}", style::class(&referrer.label))?;
            }
        }
        Ok(())
    }
}

/// Find every object that references the given object (local or from another file).
pub fn object_references<R: EnvResolver, P: TypeTreeProvider + Sync>(
    file_location: &FileLocation,
    handle: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    include_preloads: bool,
    include: &[String],
    exclude: &[String],
    include_type: &[String],
    exclude_type: &[String],
    limit: Option<usize>,
) -> Result<ObjectReferences> {
    let target_file = file_location.external_name();
    let filter = find_references::PathFilter::new(include, exclude);
    let type_filter = find_references::TypeFilter::new(include_type, exclude_type);
    let (mut referrers, truncated) = find_references::referencing_objects(
        handle.env,
        &target_file,
        path_id,
        include_preloads,
        false,
        &filter,
        &type_filter,
        limit,
    )?;
    referrers.sort();
    // The early-exit scan collects from candidates sorted before the cut-off; after sorting,
    // trimming to `limit` yields exactly the globally-first `limit` referrers.
    if let Some(limit) = limit {
        referrers.truncate(limit);
    }

    // Resolve the target's own name for the header (e.g. the MonoScript's class name).
    let target = object_name(handle, path_id).unwrap_or_else(|| format!("#{path_id}"));

    let scenes = scene_name_lookup(handle.env);
    Ok(ObjectReferences {
        target,
        path_id,
        truncated,
        referrers: referrers
            .into_iter()
            .map(|(file, _load, path_id, label)| Referrer {
                scene: scenes.get(&file).cloned(),
                file,
                path_id,
                label,
            })
            .collect(),
    })
}

/// `object <ref> references cat [--jq FILTER]`: load every referring object, [`enrich`] it, and run
/// `filter` (default `.`) over it — i.e. `object … cat --jq` applied to every instance game-wide.
/// Each result is printed as pretty JSON. Referrers are re-read through their loadable path (an
/// `archive:/…` entry for a bundle), so this spans every scene/bundle a referrer lives in.
#[allow(clippy::too_many_arguments)]
pub fn object_references_jq<R: EnvResolver + 'static, P: TypeTreeProvider + Sync + 'static>(
    file_location: &FileLocation,
    handle: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    include_preloads: bool,
    include: &[String],
    exclude: &[String],
    include_type: &[String],
    exclude_type: &[String],
    limit: Option<usize>,
    filter: &str,
    out: &mut dyn Write,
) -> Result<()> {
    use rabex_jq::jaq_json::{self, Val};
    use rabex_jq::{Enrich, QueryRunner, SceneIndex, enrich};

    let target_file = file_location.external_name();
    let path_filter = find_references::PathFilter::new(include, exclude);
    let type_filter = find_references::TypeFilter::new(include_type, exclude_type);
    let (referrers, _truncated) = find_references::referencing_objects(
        handle.env,
        &target_file,
        path_id,
        include_preloads,
        false,
        &path_filter,
        &type_filter,
        limit,
    )?;

    let env = handle.env;
    let scenes = SceneIndex::build(env)?;
    let runner = QueryRunner::new(filter)?;
    let pp = jaq_json::write::Pp {
        indent: Some("  ".to_string()),
        sep_space: true,
        ..Default::default()
    };

    let mut failed = 0usize;
    for (display, load, referrer_id, _label) in referrers {
        // One bad referrer (e.g. a query that `deref`s a null field) must not abort a game-wide
        // sweep — report it on stderr and keep going, but remember to fail the command at the end.
        let mut run = || -> Result<()> {
            let file = env.load_serialized(&load)?;
            let object = file.object_at::<Val>(referrer_id)?;
            let script = object.mono_script()?;
            let mut value = object.read()?;
            enrich(
                &mut value,
                &load,
                &file,
                Enrich {
                    scenes: Some(&scenes),
                    script: script.as_ref(),
                },
            )?;
            for result in runner.exec(env, value)? {
                let mut buf = Vec::new();
                jaq_json::write::write(&mut buf, &pp, 0, &result)
                    .expect("writing to a Vec cannot fail");
                writeln!(out, "{}", String::from_utf8_lossy(&buf))?;
            }
            Ok(())
        };
        if let Err(e) = run() {
            failed += 1;
            eprintln!(
                "{}",
                style::dim(&format!("{display} #{referrer_id}: {e:#}"))
            );
        }
    }
    // Successful results are already on stdout; still exit non-zero so a failing referrer isn't
    // silently swallowed in a pipeline.
    if failed > 0 {
        anyhow::bail!(
            "{failed} quer{} failed",
            if failed == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

/// Maps each scene's serialized location to its scene name, so referrer listings
/// can show the readable scene alongside the file: a built-in scene's `levelN`
/// file (`level1` → `Menu_Title`) and an addressables scene's bundle path
/// (`scenes_.../abyss_01.bundle` → `Abyss_01`, from the catalog — the bundle name
/// itself is lower-cased and wouldn't match `scene <name>`). The key is exactly
/// the referrer's `file` column ([`SceneSource::label`]). Best-effort: empty when
/// the scene list can't be read (e.g. a bare fixture with no `globalgamemanagers`).
fn scene_name_lookup<R: EnvResolver, P: TypeTreeProvider>(
    env: &Environment<R, P>,
) -> std::collections::HashMap<String, String> {
    crate::ctx::scenes(env)
        .map(|scenes| {
            scenes
                .into_iter()
                .map(|scene| (scene.source.label(), scene.name))
                .collect()
        })
        .unwrap_or_default()
}

/// The distinct files that reference a target object (`references -l`).
#[derive(Serialize)]
pub struct ReferencingFiles {
    /// The target object's `m_Name` (e.g. a `MonoScript`'s class name), else `#<path id>`.
    target: String,
    path_id: PathId,
    /// Whether the scan stopped early at `--limit` (so more files may exist).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    files: Vec<ReferencingFile>,
}

/// One distinct file that references the target, with its scene name when it is a
/// built-in `levelN` scene (see [`Referrer::scene`]).
#[derive(Serialize)]
pub struct ReferencingFile {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene: Option<String>,
}

impl Render for ReferencingFiles {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{}",
            style::header(&references_header(
                "file referencing",
                "files referencing",
                &self.target,
                self.files.len(),
                self.truncated,
            ))
        )?;
        for file in &self.files {
            let shown = match &file.scene {
                Some(scene) => format!("{scene} ({})", file.file),
                None => file.file.clone(),
            };
            writeln!(out, "- {}", style::name(&shown))?;
        }
        Ok(())
    }
}

/// Find the distinct files that reference the given object, stopping at the
/// first referrer per file. See `object_references` for the full per-object list.
pub fn object_referencing_files<R: EnvResolver, P: TypeTreeProvider + Sync>(
    file_location: &FileLocation,
    handle: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    include_preloads: bool,
    include: &[String],
    exclude: &[String],
    include_type: &[String],
    exclude_type: &[String],
    limit: Option<usize>,
) -> Result<ReferencingFiles> {
    let target_file = file_location.external_name();
    let filter = find_references::PathFilter::new(include, exclude);
    let type_filter = find_references::TypeFilter::new(include_type, exclude_type);
    let (referrers, truncated) = find_references::referencing_objects(
        handle.env,
        &target_file,
        path_id,
        include_preloads,
        true,
        &filter,
        &type_filter,
        limit,
    )?;

    // Collapse the (at most one per file) hits into a sorted, distinct file list.
    let mut files: Vec<String> = referrers.into_iter().map(|(file, _, _, _)| file).collect();
    files.sort();
    files.dedup();
    if let Some(limit) = limit {
        files.truncate(limit);
    }

    let target = object_name(handle, path_id).unwrap_or_else(|| format!("#{path_id}"));

    let scenes = scene_name_lookup(handle.env);
    Ok(ReferencingFiles {
        target,
        path_id,
        truncated,
        files: files
            .into_iter()
            .map(|file| ReferencingFile {
                scene: scenes.get(&file).cloned(),
                file,
            })
            .collect(),
    })
}

/// An object's non-empty `m_Name`, if it has one (best-effort: `None` when the
/// object can't be deserialized or has no/empty name).
fn object_name<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> Option<String> {
    file.object_at::<serde_json::Value>(path_id)
        .and_then(|o| o.read())
        .ok()
        .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()).map(str::to_owned))
        .filter(|name| !name.is_empty())
}

/// A label for an object that references something. Prefers its hierarchy
/// [`ComponentPath`] (`Root/Child@PlayMakerFSM`, re-usable with `object <ref>`).
/// Loose assets (not on a GameObject) have no such path, so they fall back to
/// the class (script class name for a MonoBehaviour) plus, when present, the
/// GameObject it sits on (`X (on 'GO')`) or its own `m_Name` (`X 'name'`).
/// Best-effort: falls back to the bare class, or an empty string when nothing
/// can be read.
fn referrer_label<R: EnvResolver, P: TypeTreeProvider>(
    paths: &mut crate::qualify::PathResolver<'_, R, P>,
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> String {
    if let Some(path) = paths.of(path_id) {
        return path.to_string();
    }

    let head = match component_class_and_label(file, PPtr::local(path_id)) {
        Ok((_, label)) => label,
        // The script (and thus its label) may be unresolvable; show the raw class.
        Err(_) => match file.deref(PPtr::local(path_id).typed::<()>()) {
            Ok(handle) => format!("{:?}", handle.class_id()),
            Err(_) => return String::new(),
        },
    };

    let value = file
        .object_at::<serde_json::Value>(path_id)
        .and_then(|o| o.read())
        .ok();
    // A component carries the GameObject it sits on; prefer that over its own name.
    let go_name = value
        .as_ref()
        .and_then(|v| v.get("m_GameObject"))
        .and_then(|g| g.get("m_PathID").and_then(serde_json::Value::as_i64))
        .filter(|&id| id != 0)
        .and_then(|go_id| object_name(file, go_id));
    if let Some(go) = go_name {
        return format!("{head} (on '{go}')");
    }
    match value
        .as_ref()
        .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()))
        .filter(|name| !name.is_empty())
    {
        Some(name) => format!("{head} '{name}'"),
        None => head,
    }
}

mod find_references {
    use std::io::Cursor;
    use std::path::PathBuf;

    use anyhow::{Context as _, Result};
    use rabex::files::SerializedFile;
    use rabex::objects::ClassId;
    use rabex::objects::pptr::PathId;
    use rabex::typetree::TypeTreeProvider;
    use rabex_env::Environment;
    use rabex_env::addressables::ArchivePath;
    use rabex_env::handle::SerializedFileHandle;
    use rabex_env::resolver::EnvResolver;
    use rayon::iter::ParallelBridge as _;

    /// A collected referrer: `(readable location, loadable path, path id, object label)`. The
    /// readable location is for display (a bundle path); the loadable path is what
    /// `Environment::load_serialized` opens (an `archive:/…` path for a bundle entry) so a referrer
    /// can be re-read (e.g. `references cat`).
    type Referrers = Vec<(String, String, PathId, String)>;

    /// Case-insensitive substring allow/deny list over referrer file paths.
    /// Lets the scan skip whole files before reading them.
    pub struct PathFilter {
        include: Vec<String>,
        exclude: Vec<String>,
    }

    impl PathFilter {
        pub fn new(include: &[String], exclude: &[String]) -> Self {
            let lower = |v: &[String]| v.iter().map(|s| s.to_lowercase()).collect();
            PathFilter {
                include: lower(include),
                exclude: lower(exclude),
            }
        }

        /// Whether a referrer reported under `path` should be kept: it must match
        /// some `include` (when any are set) and none of the `exclude`.
        fn accepts(&self, path: &str) -> bool {
            let path = path.to_lowercase();
            if !self.include.is_empty() && !self.include.iter().any(|i| path.contains(i)) {
                return false;
            }
            !self.exclude.iter().any(|e| path.contains(e))
        }
    }

    /// Case-insensitive substring allow/deny list over referrer object types
    /// (class name, or script class name for MonoBehaviours).
    pub struct TypeFilter {
        include: Vec<String>,
        exclude: Vec<String>,
    }

    impl TypeFilter {
        pub fn new(include: &[String], exclude: &[String]) -> Self {
            let lower = |v: &[String]| v.iter().map(|s| s.to_lowercase()).collect();
            TypeFilter {
                include: lower(include),
                exclude: lower(exclude),
            }
        }

        pub fn is_empty(&self) -> bool {
            self.include.is_empty() && self.exclude.is_empty()
        }

        /// Resolve the type label of `object` and check it against the filter.
        /// For MonoBehaviours this resolves the script class name (e.g.
        /// `PlayMakerFSM`); for other objects it uses the ClassId name (e.g.
        /// `GameObject`, `AnimationClip`).
        fn accepts<R: EnvResolver, P: TypeTreeProvider>(
            &self,
            handle: &SerializedFileHandle<'_, R, P>,
            class_id: ClassId,
            path_id: PathId,
        ) -> bool {
            if self.is_empty() {
                return true;
            }
            let type_label = if class_id == ClassId::MonoBehaviour {
                match handle.object_at::<()>(path_id) {
                    Ok(obj) => match obj.mono_script() {
                        Ok(Some(script)) => script.m_ClassName,
                        _ => "MonoBehaviour".to_owned(),
                    },
                    Err(_) => "MonoBehaviour".to_owned(),
                }
            } else {
                class_id.name().unwrap_or("Unknown").to_owned()
            };
            let type_label = type_label.to_lowercase();
            if !self.include.is_empty() && !self.include.iter().any(|i| type_label.contains(i)) {
                return false;
            }
            !self.exclude.iter().any(|e| type_label.contains(e))
        }
    }

    /// Every file whose externals list `to`. Scans both the addressables
    /// bundles and the plain serialized files in the game data directory
    /// (`levelN`, `*.assets`, `globalgamemanagers`) — scene/level files live in
    /// the latter, so omitting them misses most referrers.
    pub fn external_references<R: EnvResolver, P: TypeTreeProvider + Sync>(
        env: &Environment<R, P>,
        to: &str,
    ) -> Result<Vec<PathBuf>> {
        let mut referrers = rabex_env::utils::par_fold_reduce(
            env.addressables_bundles()?.into_iter().par_bridge(),
            |acc: &mut Vec<PathBuf>, bundle_path| {
                let bundle = env.load_addressables_bundle(&bundle_path)?;
                for entry in bundle.serialized_files() {
                    let data = bundle.read_at_entry(entry)?;
                    let file = SerializedFile::from_reader(&mut Cursor::new(data.as_slice()))?;
                    if file.externals_paths().any(|external| external == to) {
                        acc.push(bundle_path.clone());
                        break;
                    }
                }
                Ok(())
            },
        )?;

        let plain = rabex_env::utils::par_fold_reduce(
            env.game_files.serialized_files()?.into_iter().par_bridge(),
            |acc: &mut Vec<PathBuf>, path| {
                let data = env.game_files.read_path(&path)?;
                let file = SerializedFile::from_reader(&mut Cursor::new(data.as_ref()))?;
                if file.externals_paths().any(|external| external == to) {
                    acc.push(path);
                }
                Ok(())
            },
        )?;

        referrers.extend(plain);
        Ok(referrers)
    }

    /// A file that might reference the target: an addressables bundle or a plain
    /// serialized file. Reported by its readable `display` path.
    enum Candidate {
        Bundle(PathBuf),
        Plain(PathBuf),
    }

    impl Candidate {
        /// The readable path a referrer in this file is reported under — also the
        /// primary sort key, so it matches the final referrer ordering.
        fn display(&self) -> String {
            match self {
                Candidate::Bundle(p) | Candidate::Plain(p) => p.display().to_string(),
            }
        }
    }

    /// Every candidate file in the game: the addressables bundles plus the plain
    /// serialized files (`levelN`, `*.assets`, `globalgamemanagers`) — scene/level
    /// files live in the latter, so omitting them misses most referrers.
    fn candidates<R: EnvResolver, P: TypeTreeProvider + Sync>(
        env: &Environment<R, P>,
    ) -> Result<Vec<Candidate>> {
        let mut out: Vec<Candidate> = env
            .addressables_bundles()?
            .into_iter()
            .map(Candidate::Bundle)
            .collect();
        out.extend(
            env.game_files
                .serialized_files()?
                .into_iter()
                .map(Candidate::Plain),
        );
        Ok(out)
    }

    /// Scan one candidate file for objects referencing `(target_file, target_path_id)`,
    /// honouring the path `filter` (skips before reading) and the externals pre-filter.
    fn scan_candidate<R: EnvResolver, P: TypeTreeProvider>(
        env: &Environment<R, P>,
        candidate: &Candidate,
        target_file: &str,
        target_path_id: PathId,
        include_preloads: bool,
        first_only: bool,
        filter: &PathFilter,
        type_filter: &TypeFilter,
        acc: &mut Referrers,
    ) -> Result<()> {
        let display = candidate.display();
        // Filtered-out files can't contribute a referrer — skip before reading.
        if !filter.accepts(&display) {
            return Ok(());
        }
        match candidate {
            Candidate::Bundle(bundle_path) => {
                let bundle = env.load_addressables_bundle(bundle_path)?;
                let bundle_id = bundle
                    .main_serializedfile()
                    .context("bundle has no main serialized file")?
                    .path
                    .clone();
                for entry in bundle.serialized_files() {
                    let name = ArchivePath::new(&bundle_id, &entry.path).to_string();
                    let data = bundle.read_at_entry(entry)?;
                    let file = SerializedFile::from_reader(&mut Cursor::new(data.as_slice()))?;
                    if name != target_file && !file.externals_paths().any(|e| e == target_file) {
                        continue;
                    }
                    let handle = SerializedFileHandle::new(env, &file, &data);
                    scan_objects(
                        &handle,
                        &name,
                        &display,
                        target_file,
                        target_path_id,
                        include_preloads,
                        first_only,
                        type_filter,
                        acc,
                    )?;
                }
                Ok(())
            }
            Candidate::Plain(path) => {
                let data = env.game_files.read_path(path)?;
                let file = SerializedFile::from_reader(&mut Cursor::new(data.as_ref()))?;
                if display != target_file && !file.externals_paths().any(|e| e == target_file) {
                    return Ok(());
                }
                let handle = SerializedFileHandle::new(env, &file, data.as_ref());
                scan_objects(
                    &handle,
                    &display,
                    &display,
                    target_file,
                    target_path_id,
                    include_preloads,
                    first_only,
                    type_filter,
                    acc,
                )
            }
        }
    }

    /// Every object that references `(target_file, target_path_id)`, as
    /// `(readable location, path id, object label)`, plus whether the result was
    /// cut short by `limit`.
    ///
    /// Only files that list `target_file` in their externals (plus `target_file` itself, for local
    /// references) can reference the object, so the rest are skipped without scanning their objects.
    ///
    /// Without a `limit` every candidate is scanned in parallel. With one, candidates are scanned
    /// **sequentially in sorted order** so the scan can stop as soon as enough referrers are found —
    /// the early-exit means the true total is no longer known (hence the `truncated` flag).
    pub fn referencing_objects<R: EnvResolver, P: TypeTreeProvider + Sync>(
        env: &Environment<R, P>,
        target_file: &str,
        target_path_id: PathId,
        include_preloads: bool,
        first_only: bool,
        filter: &PathFilter,
        type_filter: &TypeFilter,
        limit: Option<usize>,
    ) -> Result<(Referrers, bool)> {
        let Some(limit) = limit else {
            let referrers = rabex_env::utils::par_fold_reduce(
                candidates(env)?.into_iter().par_bridge(),
                |acc: &mut Referrers, candidate| {
                    scan_candidate(
                        env,
                        &candidate,
                        target_file,
                        target_path_id,
                        include_preloads,
                        first_only,
                        filter,
                        type_filter,
                        acc,
                    )
                },
            )?;
            return Ok((referrers, false));
        };

        if limit == 0 {
            return Ok((Vec::new(), true));
        }

        // Sorted by display, the candidates' referrers come out in the final order, so the first
        // `limit` we collect are the globally-first `limit` — let us stop once we have them.
        let mut candidates = candidates(env)?;
        candidates.sort_by_cached_key(Candidate::display);

        let mut acc = Vec::new();
        // In `--files-with-matches` mode each file yields at most one shown entry, so the limit
        // counts distinct files that produced a hit rather than raw referrers.
        let mut files_hit = 0usize;
        let mut truncated = false;
        for candidate in &candidates {
            let before = acc.len();
            scan_candidate(
                env,
                candidate,
                target_file,
                target_path_id,
                include_preloads,
                first_only,
                filter,
                type_filter,
                &mut acc,
            )?;
            if acc.len() > before {
                files_hit += 1;
            }
            let reached = if first_only {
                files_hit >= limit
            } else {
                acc.len() >= limit
            };
            if reached {
                truncated = true;
                break;
            }
        }
        Ok((acc, truncated))
    }

    /// Collect objects in `handle` whose reachable PPtrs point at `(target_file, target_path_id)`,
    /// reported under `display` with a human-readable label. One entry per referring object even if
    /// it points at the target through several fields.
    fn scan_objects<R: EnvResolver, P: TypeTreeProvider>(
        handle: &SerializedFileHandle<'_, R, P>,
        name: &str,
        display: &str,
        target_file: &str,
        target_path_id: PathId,
        include_preloads: bool,
        first_only: bool,
        type_filter: &TypeFilter,
        acc: &mut Referrers,
    ) -> Result<()> {
        // `acc` accumulates across files in the fold; compare against its length
        // on entry to detect a hit from *this* file.
        let before = acc.len();
        // Resolving each referrer's component path needs the file's root scan;
        // build it once, lazily, and never for `--files-with-matches` (which
        // discards labels anyway).
        let mut paths = None;
        for object in handle.objects::<()>() {
            // Preload tables (an AssetBundle's m_PreloadTable / a PreloadData's m_Assets) list
            // the target as a load-time dependency, not a true user — skip them by default.
            if !include_preloads
                && matches!(
                    object.class_id(),
                    ClassId::AssetBundle | ClassId::PreloadData
                )
            {
                continue;
            }
            // Type filter: skip objects whose class/script name doesn't match.
            if !type_filter.accepts(handle, object.class_id(), object.path_id()) {
                continue;
            }
            for pptr in object.reachable_one()? {
                let referenced = match pptr.is_local() {
                    // a local PPtr points within `name`, so it can only hit the target object if
                    // this file *is* the target file
                    true => name,
                    false => match pptr.file_identifier(handle.file) {
                        Some(external) => external.pathName.as_str(),
                        None => continue,
                    },
                };
                if referenced == target_file && pptr.m_PathID == target_path_id {
                    let path_id = object.path_id();
                    let label = if first_only {
                        String::new()
                    } else {
                        let paths =
                            paths.get_or_insert_with(|| crate::qualify::PathResolver::new(handle));
                        super::referrer_label(paths, handle, path_id)
                    };
                    acc.push((display.to_owned(), name.to_owned(), path_id, label));
                    break;
                }
            }
            // For `--files-with-matches` one hit proves the file references the
            // target; skip its remaining objects.
            if first_only && acc.len() > before {
                break;
            }
        }
        Ok(())
    }
}
