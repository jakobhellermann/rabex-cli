//! The `preloads` file verb: an `AssetBundle`'s preload table, grouped by
//! container entry (addressable asset).

use std::io::Write;

use anyhow::{Context as _, Result};
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::PPtr;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::AssetBundle;
use serde::Serialize;

use super::bundle_name;
use crate::output::{Render, style};

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
