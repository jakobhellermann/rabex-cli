//! The `tree` file verb: the GameObject hierarchy of a serialized file.

use std::io::Write;

use anyhow::{Result, bail};
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::ClassId;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::Transform;
use serde::Serialize;

use super::Components;
use crate::component_path::ComponentPath;
use crate::output::{Render, style};
use crate::resolve::{component_class_and_label, resolve_path};

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
