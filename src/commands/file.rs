//! Verbs that operate on a single serialized file (shared by the `file`,
//! `scene` and `bundle <path> file` commands). The individual verbs live in the
//! submodules; this root holds the shared dispatch ([`run_verb`]), the target
//! selector ([`FileLocation`]) and the smaller verbs (`info`, `objects`,
//! `object … cat` / `info`, `find`).

use std::io::Write;

use anyhow::{Context as _, Result};
use rabex_env::Environment;
use rabex_env::addressables::ArchivePath;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{ClassId, PPtr};
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use serde::Serialize;

use crate::cli::{CatArgs, FileVerb, Format, InfoArgs, ObjectVerb};
use crate::component_path::ComponentPath;
use crate::output::{Render, emit, style};
use crate::resolve::{
    component_class_and_label, component_label, resolve_component_path, resolve_object_ref,
};

mod preloads;
mod references;
mod tree;

// Re-exported so `tree` / `preloads` keep their canonical `file::<name>` path
// (used by the dispatcher below and the integration tests).
pub use preloads::preloads;
pub use tree::tree;

pub enum FileLocation {
    File(String),
    Bundle { cab: String },
}
impl FileLocation {
    pub fn external_name(&self) -> String {
        match self {
            FileLocation::File(name) => name.clone(),
            FileLocation::Bundle { cab } => ArchivePath::same(cab).to_string(),
        }
    }
}

/// Run a [`FileVerb`] against a resolved serialized file. Shared by the
/// `scene`, `file` and `bundle <path> file <cab>` commands.
pub fn run_verb<R: EnvResolver + 'static, P: TypeTreeProvider + Sync + 'static>(
    file_location: FileLocation,
    file: &SerializedFileHandle<'_, R, P>,
    verb: Option<FileVerb>,
    format: Format,
) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match verb.unwrap_or_else(|| FileVerb::Info(InfoArgs::default())) {
        FileVerb::Info(args) => emit(&info(file, args.externals)?, format, &mut out),
        FileVerb::Tree(args) => {
            let components = match (args.components, args.scripts) {
                (_, true) => Components::Scripts,
                (true, _) => Components::All,
                _ => Components::None,
            };
            emit(&tree(file, args.path, components)?, format, &mut out)
        }
        FileVerb::Objects(args) => {
            emit(&list(file, args.r#type.as_deref(), true)?, format, &mut out)
        }
        FileVerb::Object(args) => {
            let path_id = resolve_object_ref(file, &args.reference)?;
            match args.verb.unwrap_or(ObjectVerb::Info) {
                ObjectVerb::Info => {
                    emit(&object_info(file, path_id)?, format, &mut out)?;
                    // `object <id>` alone only summarises; nudge towards `cat` for
                    // the deserialized fields. On stderr so it reaches both formats
                    // without polluting stdout (the JSON document stays clean).
                    eprintln!(
                        "{}",
                        style::dim("(metadata only, append `cat` to dump the object's fields)")
                    );
                    Ok(())
                }
                ObjectVerb::Cat(args) => match jq_filter(&args)? {
                    Some(filter) => object_jq(file, &file_location, path_id, &filter, &mut out),
                    None => emit(&dump_path_id(file, path_id)?, format, &mut out),
                },
                ObjectVerb::References(args) => match &args.verb {
                    // `references cat [--jq …]`: load and query every referring object.
                    Some(crate::cli::ReferencesVerb::Cat(cat)) => {
                        let filter = jq_filter(cat)?.unwrap_or_else(|| ".".to_string());
                        references::object_references_jq(
                            &file_location,
                            file,
                            path_id,
                            args.include_preloads,
                            &args.include,
                            &args.exclude,
                            &args.include_type,
                            &args.exclude_type,
                            args.limit,
                            &filter,
                            &mut out,
                        )
                    }
                    None if args.files_with_matches => emit(
                        &references::object_referencing_files(
                            &file_location,
                            file,
                            path_id,
                            args.include_preloads,
                            &args.include,
                            &args.exclude,
                            &args.include_type,
                            &args.exclude_type,
                            args.limit,
                        )?,
                        format,
                        &mut out,
                    ),
                    None => emit(
                        &references::object_references(
                            &file_location,
                            file,
                            path_id,
                            args.include_preloads,
                            &args.include,
                            &args.exclude,
                            &args.include_type,
                            &args.exclude_type,
                            args.limit,
                        )?,
                        format,
                        &mut out,
                    ),
                },
            }
        }
        FileVerb::References => emit(
            &references::references(file_location, file)?,
            format,
            &mut out,
        ),
        FileVerb::Preloads(args) => emit(
            &preloads(file, args.address.as_deref(), args.resolve)?,
            format,
            &mut out,
        ),
        FileVerb::Find(args) => emit(&find_component(file, &args.r#type)?, format, &mut out),
    }
}

/// Which components to list beneath each GameObject in a `tree`.
#[derive(Clone, Copy, PartialEq)]
pub enum Components {
    /// None.
    None,
    /// Every component.
    All,
    /// Only MonoBehaviours (by script name).
    Scripts,
}

/// Header information + type count for a serialized file.
#[derive(Serialize)]
pub struct FileInfo {
    /// `None` renders as `<unknown>`.
    unity_version: Option<String>,
    format_version: u32,
    endianness: String,
    file_size: u64,
    type_tree: bool,
    types: usize,
    objects: usize,
    externals: usize,
    /// The external files (resolved to readable bundle paths); only populated
    /// with `info --externals`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    external_bundles: Vec<String>,
}

impl Render for FileInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "{}", style::header("serialized file"))?;
        writeln!(
            out,
            "  unity version: {}",
            self.unity_version.as_deref().unwrap_or("<unknown>")
        )?;
        writeln!(out, "  format version: {}", self.format_version)?;
        writeln!(out, "  endianness: {}", self.endianness)?;
        writeln!(out, "  file size: {}", self.file_size)?;
        writeln!(out, "  type tree: {}", self.type_tree)?;
        writeln!(out, "  types: {}", self.types)?;
        writeln!(out, "  objects: {}", self.objects)?;
        writeln!(out, "  externals: {}", self.externals)?;
        for external in &self.external_bundles {
            writeln!(out, "  - {external}")?;
        }
        Ok(())
    }
}

/// Build the header information + type count for a serialized file. With
/// `externals`, the external files are listed (resolved to bundle paths); else
/// only their count is reported.
pub fn info<R: EnvResolver, P: TypeTreeProvider>(
    handle: &SerializedFileHandle<'_, R, P>,
    externals: bool,
) -> Result<FileInfo> {
    let file = handle.file;
    let header = &file.m_Header;

    // Resolve `archive:/CAB-…` externals to their readable bundle path; other
    // externals (e.g. `Library/unity default resources`) are left as-is.
    let external_bundles = if externals {
        file.externals_paths()
            .map(|external| bundle_name(handle.env, external))
            .collect()
    } else {
        Vec::new()
    };

    Ok(FileInfo {
        unity_version: file.m_UnityVersion.as_ref().map(|v| v.to_string()),
        format_version: header.m_Version,
        endianness: format!("{:?}", header.m_Endianess),
        file_size: header.m_FileSize as u64,
        type_tree: file.m_EnableTypeTree,
        types: file.m_Types.len(),
        objects: file.objects().len(),
        externals: file.externals_paths().count(),
        external_bundles,
    })
}

/// One object in an [`ObjectList`]: its path id and class. With names, also the
/// MonoBehaviour's script class name and/or the object's `m_Name`.
#[derive(Serialize)]
pub struct ObjectEntry {
    path_id: PathId,
    class: String,
    /// The script class name for a MonoBehaviour (shown as `(Script)`).
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// The objects of a serialized file (`path_id  ClassId`), optionally with names.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ObjectList(pub Vec<ObjectEntry>);

impl Render for ObjectList {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let with_label = self
            .0
            .iter()
            .any(|o| o.name.is_some() || o.script.is_some());
        for obj in &self.0 {
            let id = style::dim(&format!("{:>12}", obj.path_id));
            if !with_label {
                writeln!(out, "{id}  {}", style::class(&obj.class))?;
                continue;
            }
            let class = style::class(&format!("{:<24}", obj.class));
            // A MonoBehaviour's script as `(Script)`, then any `m_Name`.
            let mut label = String::new();
            if let Some(script) = &obj.script {
                label.push_str(&style::class(&format!("({script})")));
            }
            if let Some(name) = obj.name.as_deref().filter(|n| !n.is_empty()) {
                if !label.is_empty() {
                    label.push(' ');
                }
                label.push_str(&style::name(name));
            }
            writeln!(out, "{id}  {class}  {label}")?;
        }
        Ok(())
    }
}

/// Collect `path_id`/`ClassId` for each object, optionally filtered by class
/// name — or, for a MonoBehaviour, by its script class name (e.g. `PlayMakerFSM`).
/// Generic over the resolver so tests can drive it with an in-memory file. With
/// `names`, each object's `m_Name` is read too.
pub fn list<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    type_filter: Option<&str>,
    names: bool,
) -> Result<ObjectList> {
    let mut objects = Vec::new();
    for obj in file.objects::<()>() {
        let class_id = obj.class_id();
        let class = format!("{class_id:?}");
        let path_id = obj.path_id();

        // A MonoBehaviour's script class name — needed for the label and to let
        // `--type` match by script. `component_label` returns the class id string
        // when the script can't be resolved, so drop that case.
        let resolve_script = class_id == ClassId::MonoBehaviour && (names || type_filter.is_some());
        let script = resolve_script
            .then(|| component_label(file, PPtr::local(path_id)).ok())
            .flatten()
            .filter(|label| *label != class);

        if let Some(filter) = type_filter
            && class != *filter
            && script.as_deref() != Some(filter)
        {
            continue;
        }

        let name = names.then(|| {
            // Reading the name means deserializing the object; tolerate failures (e.g. a
            // MonoBehaviour whose script typetree isn't available) by leaving it blank.
            file.object_at::<serde_json::Value>(path_id)
                .and_then(|o| o.read())
                .ok()
                .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()).map(str::to_owned))
                .unwrap_or_default()
        });
        objects.push(ObjectEntry {
            path_id,
            class,
            // Only surfaced with names; the filter use above doesn't need it shown.
            script: if names { script } else { None },
            name,
        });
    }
    Ok(ObjectList(objects))
}

/// Read object `path_id` via its typetree, annotating PPtrs with a
/// re-`cat`-able `$ref` component path. The result is dumped as JSON.
pub fn dump_path_id<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> Result<serde_json::Value> {
    let object = file.object_at::<serde_json::Value>(path_id)?;
    let mut value = object.read()?;
    crate::qualify::qualify(file, &mut value);
    Ok(value)
}

/// The jq filter for `object … cat`, from `--jq` or `--jq-file` (or `None` for a plain dump).
fn jq_filter(args: &CatArgs) -> Result<Option<String>> {
    if let Some(filter) = &args.jq {
        Ok(Some(filter.clone()))
    } else if let Some(path) = &args.jq_file {
        Ok(Some(std::fs::read_to_string(path).with_context(|| {
            format!("reading jq file {}", path.display())
        })?))
    } else {
        Ok(None)
    }
}

/// Run a jq `filter` over object `path_id`. The object is enriched first (see [`rabex_jq::enrich`]):
/// PPtrs become `{file, path_id, class_id}` that `deref` can follow, and `_file` / `_scene` /
/// `_type` are added. Each result value is printed as pretty JSON. This is a distinct output path
/// from the `--format`-controlled dump, so `--format` doesn't apply here.
fn object_jq<R: EnvResolver + 'static, P: TypeTreeProvider + 'static>(
    file: &SerializedFileHandle<'_, R, P>,
    file_location: &FileLocation,
    path_id: PathId,
    filter: &str,
    out: &mut impl Write,
) -> Result<()> {
    use rabex_jq::jaq_json::{self, Val};
    use rabex_jq::{Enrich, QueryRunner, SceneIndex, enrich};

    let path = file_location.external_name();
    let object = file.object_at::<Val>(path_id)?;
    let script = object.mono_script()?;
    let mut value = object.read()?;

    let scenes = SceneIndex::build(file.env)?;
    enrich(
        &mut value,
        &path,
        file,
        Enrich {
            scenes: Some(&scenes),
            script: script.as_ref(),
        },
    )?;

    let runner = QueryRunner::new(filter)?;
    let pp = jaq_json::write::Pp {
        indent: Some("  ".to_string()),
        sep_space: true,
        ..Default::default()
    };
    for result in runner.exec(file.env, value)? {
        let mut buf = Vec::new();
        jaq_json::write::write(&mut buf, &pp, 0, &result).expect("writing to a Vec cannot fail");
        writeln!(out, "{}", String::from_utf8_lossy(&buf))?;
    }
    Ok(())
}

/// Map an external archive path back to its readable bundle filename, if known.
pub(super) fn bundle_name<R: EnvResolver, P: TypeTreeProvider>(
    env: &Environment<R, P>,
    archive_path: &str,
) -> String {
    let Ok(Some(addr)) = env.addressables() else {
        return archive_path.to_owned();
    };
    match ArchivePath::try_parse(std::path::Path::new(archive_path)) {
        Ok(Some(ap)) => addr
            .cab_to_bundle
            .get(ap.bundle)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| archive_path.to_owned()),
        _ => archive_path.to_owned(),
    }
}

/// One GameObject carrying a component of the searched-for type.
#[derive(Serialize)]
pub struct ComponentMatch {
    /// A re-usable component path, e.g. `_GameManager@ToolItemManager`.
    path: String,
    /// The carrying GameObject's name (the path's leaf).
    gameobject: String,
    gameobject_path_id: PathId,
    /// Path id of the matching component itself (`object #<id>` to inspect it).
    component_path_id: PathId,
}

/// Every GameObject carrying a component of the searched-for type.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ComponentMatches(pub Vec<ComponentMatch>);

impl Render for ComponentMatches {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for m in &self.0 {
            let ids = style::dim(&format!(
                "(GameObject #{}, component #{})",
                m.gameobject_path_id, m.component_path_id
            ));
            writeln!(out, "{}  {ids}", m.path)?;
        }
        Ok(())
    }
}

/// Find every GameObject in the file's hierarchy carrying a component whose label
/// (class name, or script class name for a MonoBehaviour) equals `type_name`.
/// Reuses the canonical component paths so each result round-trips through
/// `object <path>` / `tree <path>`.
pub fn find_component<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    type_name: &str,
) -> Result<ComponentMatches> {
    let mut matches = Vec::new();
    for path in crate::qualify::all_paths(file) {
        let Some(component) = &path.component else {
            continue;
        };
        if component.name != type_name {
            continue;
        }
        // Paths from `all_paths` are canonical (indices already disambiguated), so
        // resolving them back is unambiguous.
        let component_path_id = resolve_component_path(file, &path)?;
        let go_path = ComponentPath {
            segments: path.segments.clone(),
            component: None,
        };
        let gameobject_path_id = resolve_component_path(file, &go_path)?;
        let gameobject = path
            .segments
            .last()
            .map(|f| f.name.clone())
            .unwrap_or_default();
        matches.push(ComponentMatch {
            path: path.to_string(),
            gameobject,
            gameobject_path_id,
            component_path_id,
        });
    }
    Ok(ComponentMatches(matches))
}

/// Summary of an object: its class (script name for a MonoBehaviour) and, if
/// present, its `m_Name`.
#[derive(Serialize)]
pub struct ObjectInfo {
    path_id: PathId,
    class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl Render for ObjectInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "  {:<9}{}",
            "path id:",
            style::dim(&self.path_id.to_string())
        )?;
        writeln!(out, "  {:<9}{}", "class:", style::class(&self.class))?;
        if let Some(script) = &self.script {
            writeln!(out, "  {:<9}{}", "script:", style::class(script))?;
        }
        if let Some(name) = &self.name {
            writeln!(out, "  {:<9}{}", "name:", style::name(name))?;
        }
        Ok(())
    }
}

/// Build an object summary: its class (script name for a MonoBehaviour) and, if
/// present, its `m_Name`.
pub fn object_info<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> Result<ObjectInfo> {
    let (class_id, label) = component_class_and_label(file, PPtr::local(path_id))?;
    let class = format!("{class_id:?}");
    let script = (label != class).then_some(label);
    let name = file
        .object_at::<serde_json::Value>(path_id)
        .and_then(|o| o.read())
        .ok()
        .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()).map(str::to_owned))
        .filter(|name| !name.is_empty());
    Ok(ObjectInfo {
        path_id,
        class,
        script,
        name,
    })
}
