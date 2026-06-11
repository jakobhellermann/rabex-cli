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

use crate::cli::{CatArgs, FileVerb, Format, ObjArgs};
use crate::component_path::{ComponentPath, Field};

pub fn run<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    verb: FileVerb,
) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match verb {
        FileVerb::Info => info(file, &mut out),
        FileVerb::Ls(args) => list(file, args.r#type.as_deref(), &mut out),
        FileVerb::Obj(args) => dump(file, args, &mut out),
        FileVerb::Cat(args) => cat(file, args, &mut out),
        FileVerb::Tree(args) => tree(file, args.components, &mut out),
    }
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
pub fn dump<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    args: ObjArgs,
    out: &mut impl Write,
) -> Result<()> {
    dump_path_id(file, args.path_id, args.format, out)
}

/// As [`dump`], by raw path id — convenient for tests.
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

/// Print the GameObject hierarchy: each root transform (no parent) and its
/// children recursively, indented by depth. Names come from each transform's
/// GameObject; the GameObject path id is shown for `obj` follow-up. With
/// `components`, each GameObject's components are listed beneath it.
pub fn tree<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    components: bool,
    out: &mut impl Write,
) -> Result<()> {
    for transform in file.transforms() {
        let transform = transform.read()?;
        if transform.m_Father.optional().is_some() {
            continue;
        }
        print_node(file, &transform, 0, components, out)?;
    }
    Ok(())
}

fn print_node<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transform: &Transform,
    depth: usize,
    components: bool,
    out: &mut impl Write,
) -> Result<()> {
    let go = file.deref_read(transform.m_GameObject)?;
    writeln!(
        out,
        "{}{}  #{}",
        "  ".repeat(depth),
        go.m_Name,
        transform.m_GameObject.m_PathID
    )?;

    if components {
        for pair in &go.m_Component {
            writeln!(
                out,
                "{}- {}",
                "  ".repeat(depth + 1),
                component_label(file, pair.component)?
            )?;
        }
    }

    for child in &transform.m_Children {
        let child = file.deref_read(*child)?;
        print_node(file, &child, depth + 1, components, out)?;
    }
    Ok(())
}

/// A component's class name, or the script's class name for a MonoBehaviour.
pub(crate) fn component_label<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    component: PPtr,
) -> Result<String> {
    let handle = file.deref(component.typed::<()>())?;
    if handle.class_id() == ClassId::MonoBehaviour
        && let Some(script) = handle.mono_script()?
    {
        return Ok(script.m_ClassName);
    }
    Ok(format!("{:?}", handle.class_id()))
}

/// `cat` a [`ComponentPath`]: dump the selected component as JSON, or the
/// GameObject itself when no `@component` is given. PPtrs in the output carry a
/// `$ref` (see [`dump_path_id`]), so a GameObject's `m_Component` lists its
/// components by path.
pub fn cat<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    args: CatArgs,
    out: &mut impl Write,
) -> Result<()> {
    let transform = resolve_path(file, &args.path)?;
    let go = file.deref_read(transform.m_GameObject)?;

    let path_id = match &args.path.component {
        None => transform.m_GameObject.m_PathID,
        Some(selector) => {
            let mut matches = Vec::new();
            for pair in &go.m_Component {
                if component_label(file, pair.component)? == selector.name {
                    matches.push(pair.component.m_PathID);
                }
            }
            pick(matches, selector, "component")?
        }
    };
    dump_path_id(file, path_id, args.format, out)
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
