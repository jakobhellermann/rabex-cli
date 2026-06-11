//! Core logic tests — drive the extracted `list`/`dump` functions against an
//! in-memory fixture file. No filesystem, no game directory.

mod fixtures;

use fixtures::{Flat, with_handle};
use rabex_cli::cli::Format;
use rabex_cli::commands::file;

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
