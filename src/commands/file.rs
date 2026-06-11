//! Verbs that operate on a single serialized file (shared by the `file`,
//! `scene` and `bundle <path> file` commands).

use std::io::Write;

use anyhow::{Context as _, Result, bail};
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{ClassId, PPtr};
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::Transform;

use crate::cli::{FileVerb, Format, ObjectVerb};
use crate::component_path::{ComponentPath, Field, ObjectRef};

/// Run a [`FileVerb`] against a resolved serialized file. Shared by the
/// `scene`, `file` and `bundle <path> file <cab>` commands.
pub fn run_verb<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    verb: Option<FileVerb>,
) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match verb.unwrap_or(FileVerb::Info) {
        FileVerb::Info => info(file, &mut out),
        FileVerb::Tree(args) => {
            let components = match (args.components, args.scripts) {
                (_, true) => Components::Scripts,
                (true, _) => Components::All,
                _ => Components::None,
            };
            tree(file, args.path, components, &mut out)
        }
        FileVerb::Objects(args) => list(file, args.r#type.as_deref(), &mut out),
        FileVerb::Object(args) => {
            let path_id = resolve_object_ref(file, &args.reference)?;
            match args.verb.unwrap_or(ObjectVerb::Info) {
                ObjectVerb::Info => object_info(file, path_id, &mut out),
                ObjectVerb::Cat(cat) => dump_path_id(file, path_id, cat.format, &mut out),
            }
        }
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
pub fn info<R: EnvResolver, P: TypeTreeProvider>(
    handle: &SerializedFileHandle<'_, R, P>,
    out: &mut impl Write,
) -> Result<()> {
    let file = handle.file;
    let header = &file.m_Header;

    writeln!(out, "serialized file")?;
    writeln!(
        out,
        "  unity version: {}",
        file.m_UnityVersion
            .as_ref()
            .map_or_else(|| "<unknown>".to_owned(), |v| v.to_string())
    )?;
    writeln!(out, "  format version: {}", header.m_Version)?;
    writeln!(out, "  endianness: {:?}", header.m_Endianess)?;
    writeln!(out, "  file size: {}", header.m_FileSize)?;
    writeln!(out, "  type tree: {}", file.m_EnableTypeTree)?;
    writeln!(out, "  types: {}", file.m_Types.len())?;
    writeln!(out, "  objects: {}", file.objects().len())?;
    writeln!(out, "  externals: {}", file.m_Externals.len())?;
    Ok(())
}

/// Write `path_id  ClassId` for each object, optionally filtered to a single
/// class name. Generic over the resolver so tests can drive it with an
/// in-memory file.
pub fn list<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    type_filter: Option<&str>,
    out: &mut impl Write,
) -> Result<()> {
    for obj in file.objects::<()>() {
        let class_id = obj.class_id();
        if let Some(filter) = type_filter
            && format!("{class_id:?}") != *filter
        {
            continue;
        }
        writeln!(out, "{:>12}  {:?}", obj.path_id(), class_id)?;
    }
    Ok(())
}

/// Read object `path_id` via its typetree and write it in the requested format.
pub fn dump_path_id<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    format: Format,
    out: &mut impl Write,
) -> Result<()> {
    let object = file.object_at::<serde_json::Value>(path_id)?;
    let mut value = object.read()?;
    // Annotate PPtrs with a re-`cat`-able `$ref` component path.
    crate::qualify::qualify(file, &mut value);

    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut *out, &value)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

/// Print the GameObject hierarchy, indented by depth. Names come from each
/// transform's GameObject; the GameObject path id is shown for `obj` follow-up.
/// `components` selects which components to list beneath each GameObject. `root`
/// scopes the tree to one GameObject's subtree; without it, every scene root.
pub fn tree<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    root: Option<ComponentPath>,
    components: Components,
    out: &mut impl Write,
) -> Result<()> {
    if let Some(root) = root {
        if root.component.is_some() {
            bail!("tree takes a GameObject path, not a component (drop the `@…`)");
        }
        let transform = resolve_path(file, &root)?;
        print_node(file, &transform, 0, components, out)?;
        return Ok(());
    }

    for transform in file.transforms() {
        let transform = transform.read()?;
        if transform.m_Father.optional().is_some() {
            continue;
        }
        print_node(file, &transform, 0, components, out)?;
    }

    // Objects not reachable through the GameObject hierarchy (managers, assets,
    // orphaned components) — e.g. globalgamemanagers has only these.
    let outside = objects_outside_hierarchy(file)?;
    if outside > 0 {
        let noun = if outside == 1 { "object" } else { "objects" };
        writeln!(out, "{outside} {noun} outside the hierarchy")?;
    }
    Ok(())
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

/// Print a node and its subtree; returns whether anything was printed. In
/// [`Components::Scripts`] mode a GameObject with no scripts in its whole
/// subtree is pruned (returns `false`), so only paths leading to a script show.
fn print_node<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transform: &Transform,
    depth: usize,
    components: Components,
    out: &mut impl Write,
) -> Result<bool> {
    let go = file.deref_read(transform.m_GameObject)?;

    let mut component_lines = Vec::new();
    if components != Components::None {
        for pair in &go.m_Component {
            let (class_id, label) = component_class_and_label(file, pair.component)?;
            if components == Components::Scripts && class_id != ClassId::MonoBehaviour {
                continue;
            }
            component_lines.push(format!("{}- {}", "  ".repeat(depth + 1), label));
        }
    }

    // Render children first, so scripts mode can prune script-less subtrees.
    let mut children = Vec::new();
    for child in &transform.m_Children {
        let child = file.deref_read(*child)?;
        print_node(file, &child, depth + 1, components, &mut children)?;
    }

    if components == Components::Scripts && component_lines.is_empty() && children.is_empty() {
        return Ok(false);
    }

    writeln!(
        out,
        "{}{}  #{}",
        "  ".repeat(depth),
        quote_if_spaced(&go.m_Name),
        transform.m_GameObject.m_PathID
    )?;
    for line in &component_lines {
        writeln!(out, "{line}")?;
    }
    out.write_all(&children)?;
    Ok(true)
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
pub fn object_info<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    out: &mut impl Write,
) -> Result<()> {
    let (class_id, label) = component_class_and_label(file, PPtr::local(path_id))?;
    writeln!(out, "  {:<9}{path_id}", "path id:")?;
    writeln!(out, "  {:<9}{class_id:?}", "class:")?;
    if label != format!("{class_id:?}") {
        writeln!(out, "  {:<9}{label}", "script:")?;
    }
    if let Ok(value) = file
        .object_at::<serde_json::Value>(path_id)
        .and_then(|o| o.read())
        && let Some(name) = value.get("m_Name").and_then(|n| n.as_str())
        && !name.is_empty()
    {
        writeln!(out, "  {:<9}{name}", "name:")?;
    }
    Ok(())
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
