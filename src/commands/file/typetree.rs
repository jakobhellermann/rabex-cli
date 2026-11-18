//! `object … typetree`: the type tree an object is read with, and where it comes from.

use std::io::Write;

use anyhow::{Context as _, Result};
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::ClassId;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use serde::Serialize;

use crate::commands::tpk::{Node, render_node};
use crate::output::{Render, style};

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TypeTreeSource {
    /// Stored in the serialized file (`m_EnableTypeTree`).
    File,
    /// The embedded TPK, for engine classes.
    Tpk,
    /// Generated from the game's script assemblies, for MonoBehaviours.
    Script,
}

#[derive(Serialize)]
pub struct ObjectTypeTree {
    path_id: PathId,
    class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    source: TypeTreeSource,
    typetree: Node,
}

impl Render for ObjectTypeTree {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let source = match self.source {
            TypeTreeSource::File => "from the serialized file",
            TypeTreeSource::Tpk => "from the embedded tpk",
            TypeTreeSource::Script => "generated from script",
        };
        let class = match &self.script {
            Some(script) => format!("{} ({script})", self.class),
            None => self.class.clone(),
        };
        writeln!(
            out,
            "{} {}",
            style::header(&class),
            style::dim(&format!("(path id {}, type tree {source})", self.path_id))
        )?;
        render_node(&self.typetree, 0, out)
    }
}

/// The file's stored type tree if it has one, else the TPK's, or for a MonoBehaviour the one
/// generated from its script.
pub fn object_typetree<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
) -> Result<ObjectTypeTree> {
    let object = file.object_at::<()>(path_id)?;
    let info = object.object.info;

    // The script is only resolved when needed, as it can live in an external file that is
    // unreachable outside the game.
    let mut script_name = None;
    let (source, typetree) = if file.file.m_EnableTypeTree {
        let serialized_type = usize::try_from(info.m_TypeID)
            .ok()
            .and_then(|index| file.file.m_Types.get(index))
            .with_context(|| format!("object #{path_id} has invalid type id {}", info.m_TypeID))?;
        let typetree = serialized_type.m_Type.as_ref().with_context(|| {
            format!(
                "the file has type trees, but none for type id {}",
                info.m_TypeID
            )
        })?;
        (TypeTreeSource::File, Node::new(typetree))
    } else if info.m_ClassID == ClassId::MonoBehaviour {
        let script = object
            .mono_script()?
            .with_context(|| format!("MonoBehaviour #{path_id} has no MonoScript"))?;
        let typetree = file.env.generate_typetree_for_script(&script)?;
        script_name = Some(script.full_name().into_owned());
        (TypeTreeSource::Script, Node::new(typetree))
    } else {
        (TypeTreeSource::Tpk, Node::new(object.typetree()?))
    };

    Ok(ObjectTypeTree {
        path_id,
        class: format!("{:?}", info.m_ClassID),
        script: script_name,
        source,
        typetree,
    })
}
