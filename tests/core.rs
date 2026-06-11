//! Core logic tests — drive the extracted `list`/`dump` functions against an
//! in-memory fixture file. No filesystem, no game directory.

mod fixtures;

use fixtures::{Flat, with_handle};
use rabex_cli::cli::{CatArgs, Format};
use rabex_cli::commands::file;
use rabex_cli::component_path::parse as parse_path;

const PATH: &str = "level0";

#[test]
fn ls_lists_every_object() {
    let (bytes, go_ids) = Flat::new(&["Player", "Camera"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::list(file, None, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        // Two GameObjects + two Transforms.
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4);

        // Each GameObject id appears, tagged GameObject.
        for go_id in &go_ids {
            assert!(
                out.contains(&format!("{go_id:>12}  GameObject")),
                "missing GameObject line for {go_id} in:\n{out}"
            );
        }
        assert_eq!(out.matches("GameObject").count(), 2);
        assert_eq!(out.matches("Transform").count(), 2);
    });
}

#[test]
fn ls_type_filter_matches_exact_class() {
    let (bytes, _) = Flat::new(&["Player", "Camera"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::list(file, Some("GameObject"), &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert_eq!(out.lines().count(), 2);
        assert!(out.lines().all(|l| l.ends_with("GameObject")), "{out}");
    });
}

#[test]
fn ls_type_filter_unknown_class_is_empty() {
    let (bytes, _) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::list(file, Some("Texture2D"), &mut out).unwrap();
        assert_eq!(out, b"");
    });
}

#[test]
fn obj_dumps_named_gameobject_as_json() {
    let (bytes, go_ids) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::dump_path_id(file, go_ids[0], Format::Json, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();

        assert_eq!(value["m_Name"], "Player");
    });
}

#[test]
fn tree_lists_roots_at_depth_zero() {
    let (bytes, go_ids) = Flat::new(&["Player", "Camera"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::tree(file, false, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        // A flat scene: every GameObject is a root, none indented.
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                format!("Player  #{}", go_ids[0]),
                format!("Camera  #{}", go_ids[1]),
            ]
        );
    });
}

#[test]
fn tree_components_lists_each_components() {
    let (bytes, go_ids) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        file::tree(file, true, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        // The fixture gives each GameObject a single Transform component.
        assert_eq!(out, format!("Player  #{}\n  - Transform\n", go_ids[0]));
    });
}

#[test]
fn cat_lists_components_for_gameobject() {
    let (bytes, go_ids) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        let args = CatArgs {
            path: parse_path("Player").unwrap(),
            format: Format::Json,
        };
        file::cat(file, args, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        // No @component: list the GameObject's components by name.
        assert_eq!(
            out,
            format!(
                "Player  #{}  (layer 0, tag 0, active)\n  - Transform\n",
                go_ids[0]
            )
        );
    });
}

#[test]
fn cat_dumps_component_by_path() {
    let (bytes, _) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        let args = CatArgs {
            path: parse_path("Player@Transform").unwrap(),
            format: Format::Json,
        };
        file::cat(file, args, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();

        // A Transform points back at its GameObject.
        assert!(value.get("m_GameObject").is_some(), "{value}");
    });
}

#[test]
fn dump_qualifies_pptr_with_ref() {
    let (bytes, _) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        let args = CatArgs {
            path: parse_path("Player@Transform").unwrap(),
            format: Format::Json,
        };
        file::cat(file, args, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();

        // The Transform's m_GameObject PPtr gains a re-cat-able $ref.
        assert_eq!(value["m_GameObject"]["$ref"], "Player");
    });
}

#[test]
fn cat_missing_path_errors() {
    let (bytes, _) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        let args = CatArgs {
            path: parse_path("Nope").unwrap(),
            format: Format::Json,
        };
        let err = file::cat(file, args, &mut out).unwrap_err();
        assert!(err.to_string().contains("Nope"), "{err}");
    });
}

#[test]
fn obj_missing_path_id_errors() {
    let (bytes, _) = Flat::new(&["Player"]).write();

    with_handle(PATH, bytes, |file| {
        let mut out = Vec::new();
        let err = file::dump_path_id(file, 9999, Format::Json, &mut out).unwrap_err();
        assert!(
            err.to_string().contains("9999"),
            "error should mention the missing id: {err}"
        );
    });
}
