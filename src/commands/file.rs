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

use crate::cli::{FileVerb, Format, ObjectVerb};
use crate::component_path::{ComponentPath, Field, ObjectRef};
use crate::output::{Render, emit};

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
    match verb.unwrap_or(FileVerb::Info) {
        FileVerb::Info => emit(&info(file)?, format, &mut out),
        FileVerb::Tree(args) => {
            let components = match (args.components, args.scripts) {
                (_, true) => Components::Scripts,
                (true, _) => Components::All,
                _ => Components::None,
            };
            emit(&tree(file, args.path, components)?, format, &mut out)
        }
        FileVerb::Objects(args) => emit(
            &list(file, args.r#type.as_deref(), args.names)?,
            format,
            &mut out,
        ),
        FileVerb::Object(args) => {
            let path_id = resolve_object_ref(file, &args.reference)?;
            match args.verb.unwrap_or(ObjectVerb::Info) {
                ObjectVerb::Info => emit(&object_info(file, path_id)?, format, &mut out),
                ObjectVerb::Cat => emit(&dump_path_id(file, path_id)?, format, &mut out),
                ObjectVerb::References => emit(
                    &object_references(&file_location, file, path_id)?,
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
    externals: Vec<String>,
}

impl Render for FileInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "serialized file")?;
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
        writeln!(out, "  externals: {}", self.externals.len())?;
        for external in &self.externals {
            writeln!(out, "  - {external}")?;
        }
        Ok(())
    }
}

/// Build the header information + type count for a serialized file.
pub fn info<R: EnvResolver, P: TypeTreeProvider>(
    handle: &SerializedFileHandle<'_, R, P>,
) -> Result<FileInfo> {
    let file = handle.file;
    let header = &file.m_Header;

    Ok(FileInfo {
        unity_version: file.m_UnityVersion.as_ref().map(|v| v.to_string()),
        format_version: header.m_Version,
        endianness: format!("{:?}", header.m_Endianess),
        file_size: header.m_FileSize as u64,
        type_tree: file.m_EnableTypeTree,
        types: file.m_Types.len(),
        objects: file.objects().len(),
        externals: file.externals_paths().map(str::to_owned).collect(),
    })
}

/// One object in an [`ObjectList`]: its path id and class, plus its `m_Name`
/// when `objects --names` requested it.
#[derive(Serialize)]
pub struct ObjectEntry {
    path_id: PathId,
    class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// The objects of a serialized file (`path_id  ClassId`), optionally with names.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ObjectList(pub Vec<ObjectEntry>);

impl Render for ObjectList {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let with_names = self.0.iter().any(|o| o.name.is_some());
        for obj in &self.0 {
            if with_names {
                writeln!(
                    out,
                    "{:>12}  {:<24}  {}",
                    obj.path_id,
                    obj.class,
                    obj.name.as_deref().unwrap_or_default()
                )?;
            } else {
                writeln!(out, "{:>12}  {}", obj.path_id, obj.class)?;
            }
        }
        Ok(())
    }
}

/// Collect `path_id`/`ClassId` for each object, optionally filtered to a single
/// class name. Generic over the resolver so tests can drive it with an
/// in-memory file. With `names`, each object's `m_Name` is read too.
pub fn list<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    type_filter: Option<&str>,
    names: bool,
) -> Result<ObjectList> {
    let mut objects = Vec::new();
    for obj in file.objects::<()>() {
        let class_id = obj.class_id();
        let class = format!("{class_id:?}");
        if let Some(filter) = type_filter
            && class != *filter
        {
            continue;
        }
        let path_id = obj.path_id();
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
            "preload table: {} entries, container: {} entries, {} dependency bundle(s)",
            self.preload_table_len,
            self.container_len,
            self.dependencies.len()
        )?;
        for entry in &self.entries {
            writeln!(
                out,
                "\n■ {}  [{}..{}] ({} object(s))",
                entry.address,
                entry.preload_index,
                entry.preload_index + entry.preload_size,
                entry.preload_size
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
    let class = r.class.as_deref().unwrap_or("?");
    match (&r.bundle, r.local) {
        (Some(bundle), _) => format!("EXT    {class:<22} #{}  @ {bundle}", r.path_id),
        (None, true) => format!("local  {class:<22} #{}", r.path_id),
        (None, false) => format!("EXT    {class:<22} #{}", r.path_id),
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
            "{}{}  #{}",
            "  ".repeat(depth),
            quote_if_spaced(&self.name),
            self.path_id
        )?;
        for label in &self.components {
            writeln!(out, "{}- {}", "  ".repeat(depth + 1), label)?;
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
                "{} {noun} outside the hierarchy",
                self.outside_hierarchy
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

/// Resolve an object reference (raw path id or component path) to a path id.
pub fn resolve_object_ref<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    reference: &ObjectRef,
) -> Result<PathId> {
    match reference {
        ObjectRef::PathId(path_id) => Ok(*path_id),
        ObjectRef::Path(path) => resolve_component_path(file, path),
    }
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
        writeln!(out, "  {:<9}{}", "path id:", self.path_id)?;
        writeln!(out, "  {:<9}{}", "class:", self.class)?;
        if let Some(script) = &self.script {
            writeln!(out, "  {:<9}{script}", "script:")?;
        }
        if let Some(name) = &self.name {
            writeln!(out, "  {:<9}{name}", "name:")?;
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
    file: String,
    path_id: PathId,
}

/// Every object that references a target object (local or from another file).
#[derive(Serialize)]
pub struct ObjectReferences {
    target: String,
    path_id: PathId,
    referrers: Vec<Referrer>,
}

impl Render for ObjectReferences {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{} reference(s) to {}#{}:",
            self.referrers.len(),
            self.target,
            self.path_id
        )?;
        for referrer in &self.referrers {
            writeln!(out, "- {} #{}", referrer.file, referrer.path_id)?;
        }
        Ok(())
    }
}

/// Find every object that references the given object (local or from another file).
pub fn object_references<R: EnvResolver, P: TypeTreeProvider + Sync>(
    file_location: &FileLocation,
    handle: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> Result<ObjectReferences> {
    let target = file_location.external_name();
    let mut referrers = find_references::referencing_objects(handle.env, &target, path_id)?;
    referrers.sort();

    Ok(ObjectReferences {
        target,
        path_id,
        referrers: referrers
            .into_iter()
            .map(|(file, path_id)| Referrer { file, path_id })
            .collect(),
    })
}

mod find_references {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use anyhow::{Context as _, Result};
    use rabex::files::SerializedFile;
    use rabex::objects::pptr::PathId;
    use rabex::typetree::TypeTreeProvider;
    use rabex_env::Environment;
    use rabex_env::addressables::ArchivePath;
    use rabex_env::handle::SerializedFileHandle;
    use rabex_env::resolver::EnvResolver;
    use rayon::iter::ParallelBridge as _;

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

    /// Every object that references `(target_file, target_path_id)`, as `(referrer file, path id)`.
    ///
    /// Only files that list `target_file` in their externals (plus `target_file` itself, for local
    /// references) can reference the object, so the rest are skipped without scanning their objects.
    pub fn referencing_objects<R: EnvResolver, P: TypeTreeProvider + Sync>(
        env: &Environment<R, P>,
        target_file: &str,
        target_path_id: PathId,
    ) -> Result<Vec<(String, PathId)>> {
        let from_bundles = rabex_env::utils::par_fold_reduce(
            env.addressables_bundles()?.into_iter().par_bridge(),
            |acc: &mut Vec<(String, PathId)>, bundle_path| {
                let bundle = env.load_addressables_bundle(&bundle_path)?;
                let bundle_id = bundle
                    .serialized_files()
                    .find_map(|f| {
                        Path::new(&f.path)
                            .extension()
                            .is_none()
                            .then(|| f.path.clone())
                    })
                    .context("bundle has no main serialized file")?;
                for entry in bundle.serialized_files() {
                    let name = ArchivePath::new(&bundle_id, &entry.path).to_string();
                    let data = bundle.read_at_entry(entry)?;
                    let file = SerializedFile::from_reader(&mut Cursor::new(data.as_slice()))?;
                    if name != target_file && !file.externals_paths().any(|e| e == target_file) {
                        continue;
                    }
                    let handle = SerializedFileHandle::new(env, &file, &data);
                    scan_objects(&handle, &name, target_file, target_path_id, acc)?;
                }
                Ok(())
            },
        )?;

        let from_plain = rabex_env::utils::par_fold_reduce(
            env.game_files.serialized_files()?.into_iter().par_bridge(),
            |acc: &mut Vec<(String, PathId)>, path| {
                let data = env.game_files.read_path(&path)?;
                let file = SerializedFile::from_reader(&mut Cursor::new(data.as_ref()))?;
                let name = path.display().to_string();
                if name != target_file && !file.externals_paths().any(|e| e == target_file) {
                    return Ok(());
                }
                let handle = SerializedFileHandle::new(env, &file, data.as_ref());
                scan_objects(&handle, &name, target_file, target_path_id, acc)
            },
        )?;

        let mut referrers = from_bundles;
        referrers.extend(from_plain);
        Ok(referrers)
    }

    /// Collect objects in `handle` whose reachable PPtrs point at `(target_file, target_path_id)`.
    fn scan_objects<R: EnvResolver, P: TypeTreeProvider>(
        handle: &SerializedFileHandle<'_, R, P>,
        name: &str,
        target_file: &str,
        target_path_id: PathId,
        acc: &mut Vec<(String, PathId)>,
    ) -> Result<()> {
        for object in handle.objects::<()>() {
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
                    acc.push((name.to_owned(), object.path_id()));
                }
            }
        }
        Ok(())
    }
}
