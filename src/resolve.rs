//! Resolving an [`ObjectRef`] / [`ComponentPath`] to a path id (or a
//! [`Transform`]) within a serialized file, plus the component-labeling helpers.
//! Shared support code (used by the file verbs, `qualify` and `complete`), not a
//! command itself.

use anyhow::{Context as _, Result, bail};
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{ClassId, PPtr};
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::Transform;

use crate::component_path::{ComponentPath, Field, ObjectRef};

/// A component's class name, or the script's class name for a MonoBehaviour.
pub(crate) fn component_label<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    component: PPtr,
) -> Result<String> {
    Ok(component_class_and_label(file, component)?.1)
}

/// A component's class id together with its display label (the script's class
/// name for a MonoBehaviour, else the class id).
pub(crate) fn component_class_and_label<R: EnvResolver, P: TypeTreeProvider>(
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
pub(crate) fn resolve_component_path<R: EnvResolver, P: TypeTreeProvider>(
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

/// Walk the hierarchy described by `path`'s segments to the target GameObject's
/// transform.
pub(crate) fn resolve_path<R: EnvResolver, P: TypeTreeProvider>(
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
