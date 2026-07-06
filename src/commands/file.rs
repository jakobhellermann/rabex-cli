//! Verbs that operate on a single serialized file (shared by the `file`,
//! `scene` and `bundle <path> file` commands).

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use rabex_env::addressables::ArchivePath;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{ClassId, PPtr};
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::Transform;
use serde::Serialize;

use rabex_env::Environment;
use rabex_env::unity::types::AssetBundle;

use crate::cli::{FileVerb, Format, InfoArgs, ObjectVerb};
use crate::component_path::{ComponentPath, Field, ObjectRef};
use crate::output::{Render, emit, style};

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
pub fn run_verb<R: EnvResolver, P: TypeTreeProvider + Sync>(
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
                ObjectVerb::Cat => emit(&dump_path_id(file, path_id)?, format, &mut out),
                ObjectVerb::References(args) if args.files_with_matches => emit(
                    &object_referencing_files(
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
                ObjectVerb::References(args) => emit(
                    &object_references(
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
            }
        }
        FileVerb::References => emit(&references(file_location, file)?, format, &mut out),
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

/// One preloaded object: where it lives and (when resolvable) its class.
#[derive(Serialize)]
pub struct PreloadRef {
    /// `true` for objects in this CAB, `false` for objects in a dependency bundle.
    local: bool,
    path_id: PathId,
    /// Class name; `None` for externals unless `--resolve` loaded the dependency bundle.
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    /// Readable dependency-bundle name for externals (else its archive path).
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle: Option<String>,
}

/// A container entry (addressable asset) and its preload-table slice.
#[derive(Serialize)]
pub struct PreloadEntry {
    address: String,
    asset: PreloadRef,
    preload_index: i32,
    preload_size: i32,
    slice: Vec<PreloadRef>,
}

/// An `AssetBundle`'s preload table, grouped by container entry.
#[derive(Serialize)]
pub struct Preloads {
    preload_table_len: usize,
    container_len: usize,
    dependencies: Vec<String>,
    entries: Vec<PreloadEntry>,
}

impl Render for Preloads {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{}",
            style::header(&format!(
                "preload table: {} entries, container: {} entries, {} dependency bundle(s)",
                self.preload_table_len,
                self.container_len,
                self.dependencies.len()
            ))
        )?;
        for entry in &self.entries {
            writeln!(
                out,
                "\n■ {}  {}",
                style::name(&entry.address),
                style::dim(&format!(
                    "[{}..{}] ({} object(s))",
                    entry.preload_index,
                    entry.preload_index + entry.preload_size,
                    entry.preload_size
                ))
            )?;
            writeln!(out, "  asset: {}", render_ref(&entry.asset))?;
            for r in &entry.slice {
                writeln!(out, "    {}", render_ref(r))?;
            }
        }
        Ok(())
    }
}

fn render_ref(r: &PreloadRef) -> String {
    let prefix = style::dim(if r.local { "local  " } else { "EXT    " });
    let class = style::class(&format!("{:<22}", r.class.as_deref().unwrap_or("?")));
    let id = style::dim(&format!("#{}", r.path_id));
    match &r.bundle {
        Some(bundle) => format!(
            "{prefix}{class} {id}  {}",
            style::dim(&format!("@ {bundle}"))
        ),
        None => format!("{prefix}{class} {id}"),
    }
}

/// Build a [`PreloadRef`] for a PPtr. Local class ids are cheap (header lookup);
/// external class ids are only resolved when `resolve` is set (loads the dep bundle).
fn preload_ref<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    pptr: &PPtr,
    resolve: bool,
) -> PreloadRef {
    if pptr.is_local() {
        let class = file
            .file
            .get_object_info(pptr.m_PathID)
            .map(|info| format!("{:?}", info.m_ClassID));
        PreloadRef {
            local: true,
            path_id: pptr.m_PathID,
            class,
            bundle: None,
        }
    } else {
        let bundle = pptr
            .file_identifier(file.file)
            .map(|ext| bundle_name(file.env, &ext.pathName));
        let class = resolve
            .then(|| {
                file.deref::<()>(pptr.typed::<()>())
                    .ok()
                    .map(|o| format!("{:?}", o.class_id()))
            })
            .flatten();
        PreloadRef {
            local: false,
            path_id: pptr.m_PathID,
            class,
            bundle,
        }
    }
}

/// Map an external archive path back to its readable bundle filename, if known.
fn bundle_name<R: EnvResolver, P: TypeTreeProvider>(
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

/// Build the [`Preloads`] view of a file's `AssetBundle` object.
pub fn preloads<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    address_filter: Option<&str>,
    resolve: bool,
) -> Result<Preloads> {
    let ab_handle = file
        .objects_of::<AssetBundle>()
        .next()
        .context("file has no AssetBundle object (not a bundle's main CAB?)")?;
    let ab = ab_handle.read()?;

    let mut entries = Vec::new();
    for (address, info) in &ab.m_Container {
        if let Some(f) = address_filter
            && !address.contains(f)
        {
            continue;
        }
        let range = info.preload_range();
        let slice = ab
            .m_PreloadTable
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|pptr| preload_ref(file, pptr, resolve))
            .collect();
        entries.push(PreloadEntry {
            address: address.clone(),
            asset: preload_ref(file, &info.asset, resolve),
            preload_index: info.preloadIndex,
            preload_size: info.preloadSize,
            slice,
        });
    }

    Ok(Preloads {
        preload_table_len: ab.m_PreloadTable.len(),
        container_len: ab.m_Container.len(),
        dependencies: ab.m_Dependencies.clone(),
        entries,
    })
}

/// A node in the GameObject hierarchy [`Tree`].
#[derive(Serialize)]
pub struct TreeNode {
    name: String,
    path_id: PathId,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    components: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn render(&self, depth: usize, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{}{}  {}",
            "  ".repeat(depth),
            style::name(&quote_if_spaced(&self.name)),
            style::dim(&format!("#{}", self.path_id))
        )?;
        for label in &self.components {
            writeln!(out, "{}- {}", "  ".repeat(depth + 1), style::class(label))?;
        }
        for child in &self.children {
            child.render(depth + 1, out)?;
        }
        Ok(())
    }
}

/// The GameObject hierarchy of a serialized file.
#[derive(Serialize)]
pub struct Tree {
    roots: Vec<TreeNode>,
    /// Objects not reachable through the hierarchy (managers, assets, orphaned
    /// components). Always 0 when the tree is scoped to a `root`.
    outside_hierarchy: usize,
}

impl Render for Tree {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for root in &self.roots {
            root.render(0, out)?;
        }
        if self.outside_hierarchy > 0 {
            let noun = if self.outside_hierarchy == 1 {
                "object"
            } else {
                "objects"
            };
            writeln!(
                out,
                "{}",
                style::dim(&format!(
                    "{} {noun} outside the hierarchy",
                    self.outside_hierarchy
                ))
            )?;
        }
        Ok(())
    }
}

/// Build the GameObject hierarchy. Names come from each transform's GameObject;
/// the GameObject path id is recorded for `obj` follow-up. `components` selects
/// which components to attach to each GameObject. `root` scopes the tree to one
/// GameObject's subtree; without it, every scene root.
pub fn tree<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    root: Option<ComponentPath>,
    components: Components,
) -> Result<Tree> {
    if let Some(root) = root {
        if root.component.is_some() {
            bail!("tree takes a GameObject path, not a component (drop the `@…`)");
        }
        let transform = resolve_path(file, &root)?;
        let roots = build_node(file, &transform, components)?
            .into_iter()
            .collect();
        return Ok(Tree {
            roots,
            outside_hierarchy: 0,
        });
    }

    let mut roots = Vec::new();
    for transform in file.transforms() {
        let transform = transform.read()?;
        if transform.m_Father.optional().is_some() {
            continue;
        }
        if let Some(node) = build_node(file, &transform, components)? {
            roots.push(node);
        }
    }

    Ok(Tree {
        roots,
        outside_hierarchy: objects_outside_hierarchy(file)?,
    })
}

/// Count objects not covered by the GameObject hierarchy: total objects minus
/// every transform, its GameObject, and that GameObject's components.
fn objects_outside_hierarchy<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
) -> Result<usize> {
    let mut covered = std::collections::HashSet::new();
    for handle in file.transforms() {
        let transform = handle.read()?;
        covered.insert(handle.path_id());
        covered.insert(transform.m_GameObject.m_PathID);
        if let Ok(go) = file.deref_read(transform.m_GameObject) {
            for pair in &go.m_Component {
                covered.insert(pair.component.m_PathID);
            }
        }
    }
    Ok(file.file.objects().len().saturating_sub(covered.len()))
}

/// Single-quote a name containing spaces, so it can be copied into a shell.
fn quote_if_spaced(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains(' ') {
        std::borrow::Cow::Owned(format!("'{name}'"))
    } else {
        std::borrow::Cow::Borrowed(name)
    }
}

/// Build a node and its subtree, or `None` when pruned. In
/// [`Components::Scripts`] mode a GameObject with no scripts in its whole
/// subtree is pruned, so only paths leading to a script remain.
fn build_node<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transform: &Transform,
    components: Components,
) -> Result<Option<TreeNode>> {
    let go = file.deref_read(transform.m_GameObject)?;

    let mut component_labels = Vec::new();
    if components != Components::None {
        for pair in &go.m_Component {
            let (class_id, label) = component_class_and_label(file, pair.component)?;
            if components == Components::Scripts && class_id != ClassId::MonoBehaviour {
                continue;
            }
            component_labels.push(label);
        }
    }

    // Build children first, so scripts mode can prune script-less subtrees.
    let mut children = Vec::new();
    for child in &transform.m_Children {
        let child = file.deref_read(*child)?;
        if let Some(node) = build_node(file, &child, components)? {
            children.push(node);
        }
    }

    if components == Components::Scripts && component_labels.is_empty() && children.is_empty() {
        return Ok(None);
    }

    Ok(Some(TreeNode {
        name: go.m_Name,
        path_id: transform.m_GameObject.m_PathID,
        components: component_labels,
        children,
    }))
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

/// A component's class name, or the script's class name for a MonoBehaviour.
pub(crate) fn component_label<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    component: PPtr,
) -> Result<String> {
    Ok(component_class_and_label(file, component)?.1)
}

/// A component's class id together with its display label (the script's class
/// name for a MonoBehaviour, else the class id).
fn component_class_and_label<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    component: PPtr,
) -> Result<(ClassId, String)> {
    let handle = file.deref(component.typed::<()>())?;
    let class_id = handle.class_id();
    if class_id == ClassId::MonoBehaviour
        && let Some(script) = handle.mono_script()?
    {
        return Ok((class_id, script.m_ClassName));
    }
    Ok((class_id, format!("{class_id:?}")))
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

/// Resolve an object reference (raw path id or component path) to a path id.
pub fn resolve_object_ref<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    reference: &ObjectRef,
) -> Result<PathId> {
    match reference {
        ObjectRef::PathId(path_id) => Ok(*path_id),
        // A single bare name (no `/` segments, no `@component`) can also select a
        // non-GameObject object by its `m_Name` (e.g. a `MonoScript` named
        // `PlayMakerFSM`) or, for class-typed singletons, by its class name (e.g.
        // `TagManager` in `globalgamemanagers`). Try the GameObject hierarchy
        // first, then fall back to a name/class match.
        ObjectRef::Path(path) => match path.segments.as_slice() {
            [field] if path.component.is_none() => match resolve_component_path(file, path) {
                Ok(path_id) => Ok(path_id),
                Err(_) => resolve_object_by_name(file, field),
            },
            _ => resolve_component_path(file, path),
        },
    }
}

/// Resolve a single object by its `m_Name`, or — when nothing is named that —
/// by its class name (for class-typed singletons like `TagManager`). Scans every
/// object in the file; `field.index` disambiguates when several match.
fn resolve_object_by_name<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    field: &Field,
) -> Result<PathId> {
    let mut by_name = Vec::new();
    let mut by_class = Vec::new();
    for obj in file.objects::<()>() {
        let path_id = obj.path_id();
        if format!("{:?}", obj.class_id()) == field.name {
            by_class.push(path_id);
        }
        // Reading the name deserializes the object; tolerate failures (e.g. a
        // MonoBehaviour whose script typetree isn't available) by skipping it.
        let name = file
            .object_at::<serde_json::Value>(path_id)
            .and_then(|o| o.read())
            .ok()
            .and_then(|v| v.get("m_Name").and_then(|n| n.as_str()).map(str::to_owned));
        if name.as_deref() == Some(field.name.as_str()) {
            by_name.push(path_id);
        }
    }
    // Prefer name matches; class matches are the fallback for unnamed singletons.
    let matches = if by_name.is_empty() {
        by_class
    } else {
        by_name
    };
    pick(matches, field, "object")
}

/// Resolve a [`ComponentPath`] to the path id of its GameObject, or of the
/// selected `@component` on it.
fn resolve_component_path<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path: &ComponentPath,
) -> Result<PathId> {
    let transform = resolve_path(file, path)?;
    let go = file.deref_read(transform.m_GameObject)?;
    match &path.component {
        None => Ok(transform.m_GameObject.m_PathID),
        Some(selector) => {
            let mut matches = Vec::new();
            for pair in &go.m_Component {
                if component_label(file, pair.component)? == selector.name {
                    matches.push(pair.component.m_PathID);
                }
            }
            pick(matches, selector, "component")
        }
    }
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

/// Walk the hierarchy described by `path`'s segments to the target GameObject's
/// transform.
fn resolve_path<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path: &ComponentPath,
) -> Result<Transform> {
    let mut roots = Vec::new();
    for transform in file.transforms() {
        let transform = transform.read()?;
        if transform.m_Father.optional().is_none() {
            roots.push(transform);
        }
    }

    let (first, rest) = path.segments.split_first().expect("at least one segment");
    let mut current = pick_transform(file, roots, first, "root object")?;
    for segment in rest {
        let children = current
            .m_Children
            .iter()
            .map(|child| file.deref_read(*child))
            .collect::<Result<Vec<_>>>()?;
        current = pick_transform(file, children, segment, "child object")?;
    }
    Ok(current)
}

/// Pick the transform whose GameObject name matches `field` from `transforms`.
fn pick_transform<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transforms: Vec<Transform>,
    field: &Field,
    kind: &str,
) -> Result<Transform> {
    let mut matches = Vec::new();
    for transform in transforms {
        let name = file.deref_read(transform.m_GameObject)?.m_Name;
        if name == field.name {
            matches.push(transform);
        }
    }
    pick(matches, field, kind)
}

/// Select one match by `field.index`, or the sole match if no index was given.
fn pick<T>(matches: Vec<T>, field: &Field, kind: &str) -> Result<T> {
    let n = matches.len();
    match field.index {
        Some(index) => matches.into_iter().nth(index).with_context(|| {
            format!(
                "{kind} '{}': index {index} out of range ({n} match(es))",
                field.name
            )
        }),
        None => match n {
            0 => bail!("no {kind} matching '{}'", field.name),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => bail!(
                "{kind} '{}' is ambiguous ({n} matches); add ':<index>' (0..{})",
                field.name,
                n - 1
            ),
        },
    }
}

/// The files that reference a serialized file (via their externals list).
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
            .map(|(file, path_id, label)| Referrer {
                scene: scenes.get(&file).cloned(),
                file,
                path_id,
                label,
            })
            .collect(),
    })
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
    let mut files: Vec<String> = referrers.into_iter().map(|(file, _, _)| file).collect();
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

    /// A collected referrer: `(readable location, path id, object label)`.
    type Referrers = Vec<(String, PathId, String)>;

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
                    acc.push((display.to_owned(), path_id, label));
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
