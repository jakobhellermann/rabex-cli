//! In-memory Unity SerializedFile fixtures for tests.
//!
//! `SerializedFileBuilder` assembles tiny scenes (GameObjects + Transforms)
//! into a `Vec<u8>` without touching disk. `MemResolver` then re-opens those
//! bytes through a real rabex `Environment`, so the production load/read code
//! runs against a genuine `SerializedFileHandle`.
//!
//! Adapted from steam-multiversion-viewer's fixtures, trimmed to what the cli
//! tests need.
#![allow(dead_code)]

use std::io::Cursor;

use rabex_env::Environment;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::UnityVersion;
use rabex_env::rabex::files::serializedfile::build_common_offset_map;
use rabex_env::rabex::files::serializedfile::builder::SerializedFileBuilder;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::objects::{PPtr, TypedPPtr};
use rabex_env::rabex::tpk::TpkTypeTreeBlob;
use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::resolver::MemResolver;
use rabex_env::unity::types::{ComponentPair, GameObject, Transform};

/// Unity version every fixture is built with. The embedded TPK has full
/// coverage for it.
pub const TEST_UNITY_VERSION: &str = "2022.3.0f1";

/// Open scene bytes via a fresh `Environment` and hand the resulting handle to
/// `f`. Closure-shaped so the env outlives the handle.
pub fn with_handle<R>(
    path: &str,
    bytes: Vec<u8>,
    f: impl FnOnce(&SerializedFileHandle<'_, MemResolver, TypeTreeCache<TpkTypeTreeBlob>>) -> R,
) -> R {
    let resolver = MemResolver::single(path, bytes);
    let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
    let env = Environment::new(resolver, tpk);
    let handle = env.load_serialized(path).unwrap();
    f(&handle)
}

/// A flat scene: one GameObject + Transform per name, written in order.
/// Returns the bytes plus the path ids assigned to each GameObject so tests
/// can assert against known ids.
pub struct Flat {
    names: Vec<&'static str>,
}

impl Flat {
    pub fn new(names: &[&'static str]) -> Self {
        Flat {
            names: names.to_vec(),
        }
    }

    /// Build the file. Returns `(bytes, gameobject_path_ids)`.
    pub fn write(&self) -> (Vec<u8>, Vec<PathId>) {
        let unity_version: UnityVersion = TEST_UNITY_VERSION.parse().unwrap();
        let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
        let common = build_common_offset_map(&tpk.inner, &unity_version);
        let mut sfb = SerializedFileBuilder::new(&unity_version, &tpk, &common, true);

        let mut go_ids = Vec::new();
        for name in &self.names {
            let go_id = sfb.get_next_path_id();
            let transform_id = sfb.get_next_path_id();

            let go = GameObject {
                m_Component: vec![ComponentPair {
                    component: PPtr::local(transform_id),
                }],
                m_Layer: 0,
                m_Name: (*name).to_owned(),
                m_Tag: 0,
                m_IsActive: true,
            };
            sfb.add_object_at(go_id, &go).unwrap();

            let transform = Transform {
                m_GameObject: TypedPPtr::local(go_id),
                m_LocalRotation: (0.0, 0.0, 0.0, 1.0),
                m_LocalPosition: (0.0, 0.0, 0.0),
                m_LocalScale: (1.0, 1.0, 1.0),
                m_Children: Vec::new(),
                m_Father: TypedPPtr::null(),
            };
            sfb.add_object_at(transform_id, &transform).unwrap();

            go_ids.push(go_id);
        }

        (sfb.write_vec().unwrap(), go_ids)
    }
}

/// Wrap raw serialized-file bytes into a minimal uncompressed UnityFS bundle
/// holding a single serialized entry named `entry_name`.
pub fn bundle_with_serialized(entry_name: &str, serialized: &[u8]) -> Vec<u8> {
    use rabex_env::rabex::files::bundlefile::CompressionType;
    use rabex_env::rabex::files::bundlefile::builder::BundleFileBuilder;

    let unity_version: UnityVersion = TEST_UNITY_VERSION.parse().unwrap();
    let mut builder = BundleFileBuilder::unityfs(7, &unity_version);
    builder.add_file(entry_name, serialized).unwrap();

    let mut out = Cursor::new(Vec::new());
    builder.write(&mut out, CompressionType::None).unwrap();
    out.into_inner()
}
