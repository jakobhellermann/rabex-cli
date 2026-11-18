//! The `tpk` commands: look up Unity engine class type trees in the embedded TPK.

use std::io::Write;

use anyhow::{Context as _, Result, bail};
use rabex_env::rabex::UnityVersion;
use rabex_env::rabex::objects::ClassId;
use rabex_env::rabex::tpk::{TpkTypeTreeBlob, TpkUnityClassFlags, UnityClass};
use rabex_env::rabex::typetree::TypeTreeNode;
use serde::Serialize;

use crate::cli::{Format, TpkArgs, TpkVerb};
use crate::output::{Render, emit, style};

pub fn run(args: TpkArgs, format: Format) -> Result<()> {
    let tpk = TpkTypeTreeBlob::embedded();
    let version = match args.unity_version {
        Some(version) => version,
        None => latest_version(&tpk)?,
    };
    let out = &mut std::io::stdout().lock();
    match args.verb {
        TpkVerb::Class(args) => emit(&class(&tpk, &args.name, &version)?, format, out),
        TpkVerb::References(args) => emit(&references(&tpk, &args.r#type, &version)?, format, out),
    }
}

pub fn latest_version(tpk: &TpkTypeTreeBlob) -> Result<UnityVersion> {
    tpk.versions
        .iter()
        .max()
        .cloned()
        .context("the TPK contains no unity versions")
}

/// The name of each class the TPK knows, with the versions in which its type tree changed.
pub fn classes(tpk: &TpkTypeTreeBlob) -> Vec<(&str, ClassId, Vec<&UnityVersion>)> {
    let mut classes: Vec<_> = tpk
        .class_information
        .iter()
        .filter_map(|(&class_id, versions)| {
            let name = versions
                .iter()
                .rev()
                .find_map(|(_, class)| class.as_ref())?;
            let changes = versions.iter().map(|(version, _)| version).collect();
            Some((class_name(tpk, name), class_id, changes))
        })
        .collect();
    classes.sort_by_key(|&(name, ..)| name);
    classes
}

fn class_name<'a>(tpk: &'a TpkTypeTreeBlob, class: &UnityClass) -> &'a str {
    &tpk.string_buffer[class.name as usize]
}

/// The class entry in effect at `version`, and the version it was introduced in.
fn class_at<'a>(
    tpk: &'a TpkTypeTreeBlob,
    class_id: ClassId,
    version: &UnityVersion,
) -> Option<(&'a UnityVersion, &'a UnityClass)> {
    let (since, class) = tpk.class_information[&class_id]
        .iter()
        .rev()
        .find(|(since, _)| version >= since)?;
    Some((since, class.as_ref()?))
}

#[derive(Serialize)]
pub struct ClassTypeTree {
    class: String,
    class_id: i32,
    unity_version: String,
    since: String,
    typetree: Node,
}

#[derive(Serialize)]
pub struct Node {
    r#type: String,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Node>,
}

impl Node {
    pub fn new(node: &TypeTreeNode) -> Node {
        Node {
            r#type: node.m_Type.clone(),
            name: node.m_Name.clone(),
            children: node.children.iter().map(Node::new).collect(),
        }
    }
}

impl Render for ClassTypeTree {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{} {}",
            style::header(&self.class),
            style::dim(&format!(
                "(class id {}, layout since {})",
                self.class_id, self.since
            ))
        )?;
        render_node(&self.typetree, 0, out)
    }
}

pub fn render_node(node: &Node, depth: usize, out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "{:indent$}{} {}",
        "",
        style::class(&node.r#type),
        node.name,
        indent = depth * 2
    )?;
    for child in &node.children {
        render_node(child, depth + 1, out)?;
    }
    Ok(())
}

pub fn class(tpk: &TpkTypeTreeBlob, name: &str, version: &UnityVersion) -> Result<ClassTypeTree> {
    let (class_name, class_id, _) = classes(tpk)
        .into_iter()
        .find(|(class_name, ..)| class_name.eq_ignore_ascii_case(name))
        .with_context(|| format!("the TPK has no class '{name}'"))?;
    let (since, unity_class) = class_at(tpk, class_id, version)
        .with_context(|| format!("{class_name} does not exist in {version}"))?;
    if unity_class.flags.contains(TpkUnityClassFlags::IS_ABSTRACT) {
        bail!("{class_name} is abstract and has no type tree of its own");
    }
    let typetree = tpk
        .get_typetree_node(class_id, version)
        .with_context(|| format!("{class_name} has no release type tree in {version}"))?;
    Ok(ClassTypeTree {
        class: class_name.to_owned(),
        class_id: class_id.0,
        unity_version: version.to_string(),
        since: since.to_string(),
        typetree: Node::new(&typetree),
    })
}

#[derive(Serialize)]
pub struct TypeReferences {
    r#type: String,
    unity_version: String,
    references: Vec<TypeReference>,
}

#[derive(Serialize)]
struct TypeReference {
    class: String,
    path: String,
}

impl Render for TypeReferences {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(
            out,
            "{}",
            style::header(&format!(
                "{} field(s) of type {} in {}",
                self.references.len(),
                self.r#type,
                self.unity_version
            ))
        )?;
        let width = self
            .references
            .iter()
            .map(|r| r.class.len())
            .max()
            .unwrap_or(0);
        for reference in &self.references {
            let class = format!("{:<width$}", reference.class);
            writeln!(out, "  {}  {}", style::class(&class), reference.path)?;
        }
        Ok(())
    }
}

/// Every field, in every class, whose type is exactly `type_name`.
pub fn references(
    tpk: &TpkTypeTreeBlob,
    type_name: &str,
    version: &UnityVersion,
) -> Result<TypeReferences> {
    let mut references = Vec::new();
    for (class_name, class_id, _) in classes(tpk) {
        if class_at(tpk, class_id, version).is_none() {
            continue;
        }
        let Some(typetree) = tpk.get_typetree_node(class_id, version) else {
            continue;
        };
        let mut paths = Vec::new();
        for child in &typetree.children {
            collect_paths(child, type_name, child.m_Name.clone(), &mut paths);
        }
        references.extend(paths.into_iter().map(|path| TypeReference {
            class: class_name.to_owned(),
            path,
        }));
    }
    Ok(TypeReferences {
        r#type: type_name.to_owned(),
        unity_version: version.to_string(),
        references,
    })
}

fn collect_paths(node: &TypeTreeNode, type_name: &str, path: String, out: &mut Vec<String>) {
    if node.m_Type == type_name {
        out.push(path);
        return;
    }
    for child in &node.children {
        let child_path = match child.m_Name.as_str() {
            "Array" => path.clone(),
            "data" => format!("{path}[]"),
            name => format!("{path}.{name}"),
        };
        collect_paths(child, type_name, child_path, out);
    }
}
