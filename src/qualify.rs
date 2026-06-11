//! Annotating PPtrs in a dumped object's JSON with a human-readable, and
//! re-`cat`-able, `$ref` component path.
//!
//! A serialized PPtr is `{ "m_FileID": .., "m_PathID": .. }`. For every local
//! one that resolves to a scene-hierarchy object we add a `$ref` field holding
//! the [`ComponentPath`] addressing it — `Root/Child@SpriteRenderer`, with
//! disambiguating `:index` only where names repeat — so it round-trips back
//! through `cat`.

use std::collections::HashMap;

use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{ClassId, PPtr, TypedPPtr};
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::{Component, GameObject, Transform};
use serde_json::Value;

use crate::commands::file::component_label;
use crate::component_path::{ComponentPath, Field};

/// Walk a dumped object's JSON and add a `$ref` to every local PPtr that
/// resolves to a scene object. Best-effort: a PPtr that can't be resolved is
/// left untouched.
pub fn qualify<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    value: &mut Value,
) {
    let mut cx = Cx {
        file,
        roots: roots(file),
        cache: HashMap::new(),
    };
    walk(&mut cx, value);
}

struct Cx<'a, R, P> {
    file: &'a SerializedFileHandle<'a, R, P>,
    /// All root transforms as (transform path id, owning GameObject name).
    roots: Vec<(PathId, String)>,
    /// Memoised `$ref` per target path id (None = not a hierarchy object).
    cache: HashMap<PathId, Option<String>>,
}

fn walk<R: EnvResolver, P: TypeTreeProvider>(cx: &mut Cx<'_, R, P>, value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(path_id) = local_pptr(map) {
                if let Some(reference) = ref_for(cx, path_id) {
                    map.insert("$ref".to_owned(), Value::String(reference));
                }
                return;
            }
            for child in map.values_mut() {
                walk(cx, child);
            }
        }
        Value::Array(items) => {
            for child in items {
                walk(cx, child);
            }
        }
        _ => {}
    }
}

/// The path id of a local, non-null PPtr map (`{m_FileID: 0, m_PathID: N}`).
fn local_pptr(map: &serde_json::Map<String, Value>) -> Option<PathId> {
    if map.len() != 2 {
        return None;
    }
    let file_id = map.get("m_FileID")?.as_i64()?;
    let path_id = map.get("m_PathID")?.as_i64()?;
    (file_id == 0 && path_id != 0).then_some(path_id)
}

/// Memoised `$ref` string for a target path id.
fn ref_for<R: EnvResolver, P: TypeTreeProvider>(
    cx: &mut Cx<'_, R, P>,
    target: PathId,
) -> Option<String> {
    if let Some(cached) = cx.cache.get(&target) {
        return cached.clone();
    }
    let reference = build_path(cx, target)
        .unwrap_or(None)
        .map(|p| p.to_string());
    cx.cache.insert(target, reference.clone());
    reference
}

/// Build the [`ComponentPath`] addressing `target`, or `None` if it is not a
/// GameObject or a component on one (e.g. a loose asset).
fn build_path<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    target: PathId,
) -> anyhow::Result<Option<ComponentPath>> {
    let class = cx.file.object_at::<()>(target)?.class_id();

    let (go_id, component) = if class == ClassId::GameObject {
        (target, None)
    } else {
        // Components carry an `m_GameObject`; assets without one aren't in the
        // hierarchy and get no `$ref`.
        let Ok(component) = cx.file.deref_read(TypedPPtr::<Component>::local(target)) else {
            return Ok(None);
        };
        let Some(go) = component.m_GameObject.optional() else {
            return Ok(None);
        };
        let label = component_label(cx.file, PPtr::local(target))?;
        let index = component_index(cx, go.m_PathID, target, &label)?;
        (go.m_PathID, Some(Field { name: label, index }))
    };

    let Some(segments) = go_segments(cx, go_id)? else {
        return Ok(None);
    };
    Ok(Some(ComponentPath {
        segments,
        component,
    }))
}

/// Index of `target` among `go`'s components sharing its label, or `None` if it
/// is the only one of that label.
fn component_index<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    go_id: PathId,
    target: PathId,
    label: &str,
) -> anyhow::Result<Option<usize>> {
    let go = cx.file.deref_read(TypedPPtr::<GameObject>::local(go_id))?;
    let mut same = Vec::new();
    for pair in &go.m_Component {
        if component_label(cx.file, pair.component)? == label {
            same.push(pair.component.m_PathID);
        }
    }
    Ok(disambiguate(&same, target))
}

/// The hierarchy segments (root first) addressing the GameObject `go_id`, or
/// `None` if it has no Transform (not part of the scene hierarchy).
fn go_segments<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    go_id: PathId,
) -> anyhow::Result<Option<Vec<Field>>> {
    let Some(transform_id) = gameobject_transform(cx, go_id)? else {
        return Ok(None);
    };

    // Walk up the parent chain, root last.
    let mut chain = Vec::new();
    let mut id = transform_id;
    loop {
        let transform = cx.file.deref_read(TypedPPtr::<Transform>::local(id))?;
        let father = transform.m_Father.optional();
        chain.push((id, transform));
        match father {
            Some(father) => id = father.m_PathID,
            None => break,
        }
    }
    chain.reverse();

    let mut segments = Vec::with_capacity(chain.len());
    for (i, (id, transform)) in chain.iter().enumerate() {
        let name = transform_go_name(cx, transform)?;
        let siblings = if i == 0 {
            self_named_roots(cx, &name)
        } else {
            named_children(cx, &chain[i - 1].1, &name)?
        };
        segments.push(Field {
            name,
            index: disambiguate(&siblings, *id),
        });
    }
    Ok(Some(segments))
}

/// The path id of a GameObject's (Rect)Transform component, if any.
fn gameobject_transform<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    go_id: PathId,
) -> anyhow::Result<Option<PathId>> {
    let go = cx.file.deref_read(TypedPPtr::<GameObject>::local(go_id))?;
    for pair in &go.m_Component {
        let class = cx.file.deref(pair.component.typed::<()>())?.class_id();
        if class == ClassId::Transform || class == ClassId::RectTransform {
            return Ok(Some(pair.component.m_PathID));
        }
    }
    Ok(None)
}

fn transform_go_name<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    transform: &Transform,
) -> anyhow::Result<String> {
    Ok(cx.file.deref_read(transform.m_GameObject)?.m_Name)
}

/// Root transform ids whose GameObject is named `name`.
fn self_named_roots<R, P>(cx: &Cx<'_, R, P>, name: &str) -> Vec<PathId> {
    cx.roots
        .iter()
        .filter(|(_, n)| n == name)
        .map(|(id, _)| *id)
        .collect()
}

/// Child transform ids of `parent` whose GameObject is named `name`.
fn named_children<R: EnvResolver, P: TypeTreeProvider>(
    cx: &Cx<'_, R, P>,
    parent: &Transform,
    name: &str,
) -> anyhow::Result<Vec<PathId>> {
    let mut matches = Vec::new();
    for child in &parent.m_Children {
        let transform = cx.file.deref_read(*child)?;
        if transform_go_name(cx, &transform)? == name {
            matches.push(child.m_PathID);
        }
    }
    Ok(matches)
}

/// `Some(position)` of `target` among `matches` when there is more than one,
/// else `None` (a sole match needs no index).
fn disambiguate(matches: &[PathId], target: PathId) -> Option<usize> {
    if matches.len() <= 1 {
        return None;
    }
    matches.iter().position(|id| *id == target)
}

/// All root transforms as (path id, owning GameObject name).
fn roots<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
) -> Vec<(PathId, String)> {
    let mut roots = Vec::new();
    for handle in file.transforms() {
        let path_id = handle.path_id();
        let Ok(transform) = handle.read() else {
            continue;
        };
        if transform.m_Father.optional().is_some() {
            continue;
        }
        if let Ok(go) = file.deref_read(transform.m_GameObject) {
            roots.push((path_id, go.m_Name));
        }
    }
    roots
}

/// A hierarchy node: a Transform plus its owning GameObject's name.
struct Node {
    transform: Transform,
    name: String,
}

/// Every addressable component path in the file — each GameObject and each of
/// its components — for `cat` completion. Best-effort: unreadable nodes are
/// skipped. The shell does the prefix matching, so we just enumerate.
pub fn all_paths<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
) -> Vec<ComponentPath> {
    let mut roots = Vec::new();
    for handle in file.transforms() {
        let Ok(transform) = handle.read() else {
            continue;
        };
        if transform.m_Father.optional().is_some() {
            continue;
        }
        if let Some(node) = node_of(file, transform) {
            roots.push(node);
        }
    }

    let mut out = Vec::new();
    walk_level(file, &[], roots, &mut out);
    out
}

fn node_of<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transform: Transform,
) -> Option<Node> {
    let name = file.deref_read(transform.m_GameObject).ok()?.m_Name;
    Some(Node { transform, name })
}

/// Emit paths for every node at this level (siblings) and recurse into each.
fn walk_level<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    prefix: &[Field],
    nodes: Vec<Node>,
    out: &mut Vec<ComponentPath>,
) {
    let mut counter = Counter::new(nodes.iter().map(|n| n.name.as_str()));
    for node in &nodes {
        let mut segments = prefix.to_vec();
        segments.push(Field {
            name: node.name.clone(),
            index: counter.next_index(&node.name),
        });

        out.push(ComponentPath {
            segments: segments.clone(),
            component: None,
        });
        emit_components(file, &segments, &node.transform, out);

        let children = node
            .transform
            .m_Children
            .iter()
            .filter_map(|child| file.deref_read(*child).ok())
            .filter_map(|transform| node_of(file, transform))
            .collect();
        walk_level(file, &segments, children, out);
    }
}

/// Emit a `…@Component` path for each component of `transform`'s GameObject.
fn emit_components<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    segments: &[Field],
    transform: &Transform,
    out: &mut Vec<ComponentPath>,
) {
    let Ok(go) = file.deref_read(transform.m_GameObject) else {
        return;
    };
    let labels: Vec<String> = go
        .m_Component
        .iter()
        .filter_map(|pair| component_label(file, pair.component).ok())
        .collect();

    let mut counter = Counter::new(labels.iter().map(String::as_str));
    for label in &labels {
        let index = counter.next_index(label);
        out.push(ComponentPath {
            segments: segments.to_vec(),
            component: Some(Field {
                name: label.clone(),
                index,
            }),
        });
    }
}

/// Assigns disambiguating indices: `None` for names that occur once, else a
/// running 0-based index per occurrence.
struct Counter {
    totals: HashMap<String, usize>,
    seen: HashMap<String, usize>,
}

impl Counter {
    fn new<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let mut totals = HashMap::new();
        for name in names {
            *totals.entry(name.to_owned()).or_insert(0) += 1;
        }
        Counter {
            totals,
            seen: HashMap::new(),
        }
    }

    fn next_index(&mut self, name: &str) -> Option<usize> {
        if self.totals.get(name).copied().unwrap_or(0) <= 1 {
            return None;
        }
        let seen = self.seen.entry(name.to_owned()).or_insert(0);
        let index = *seen;
        *seen += 1;
        Some(index)
    }
}
