//! CLI-level tests — invoke the `rabex` binary and assert on stdout/stderr and
//! exit codes. Fixtures are written into ephemeral temp dirs; for game-context
//! tests the layout mimics a Unity game (`*_Data/`). Nothing binary is
//! committed; the bytes are synthesized per test.

mod fixtures;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use fixtures::Flat;
use tempfile::TempDir;

/// Write a flat scene into `<tmp>/Game_Data/<file>` and return (tempdir, full
/// path, gameobject ids). The tempdir guard must outlive the file.
fn game_with_file(file: &str, names: &[&'static str]) -> (TempDir, PathBuf, Vec<i64>) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();

    let (bytes, go_ids) = Flat::new(names).write();
    let path = data_dir.join(file);
    std::fs::write(&path, bytes).unwrap();

    (tmp, path, go_ids)
}

/// Write a standalone serialized file (no game around it) and return its path.
fn standalone_file(names: &[&'static str]) -> (TempDir, PathBuf, Vec<i64>) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("plain");
    std::fs::create_dir(&dir).unwrap();
    let (bytes, go_ids) = Flat::new(names).write();
    let path = dir.join("level0");
    std::fs::write(&path, bytes).unwrap();
    (tmp, path, go_ids)
}

/// Write a standalone `.bundle` (no surrounding game) and return its path.
fn standalone_bundle(names: &[&'static str]) -> (TempDir, PathBuf, Vec<i64>) {
    let tmp = TempDir::new().unwrap();
    let (serialized, go_ids) = Flat::new(names).write();
    let bytes = fixtures::bundle_with_serialized("CAB-test", &serialized);
    let path = tmp.path().join("loose.bundle");
    std::fs::write(&path, bytes).unwrap();
    (tmp, path, go_ids)
}

fn rabex() -> Command {
    Command::cargo_bin("rabex").unwrap()
}

// -----------------------------------------------------------------------------
// ls
// -----------------------------------------------------------------------------

#[test]
fn ls_file_lists_objects() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("--file")
        .arg(&path)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2))
        .stdout(predicates::str::contains("Transform").count(2));
}

#[test]
fn ls_type_filter() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("--file")
        .arg(&path)
        .args(["ls", "--type", "GameObject"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2))
        .stdout(predicates::str::contains("Transform").count(0));
}

/// `--game-dir DIR --file NAME` resolves the file relative to the game.
#[test]
fn ls_file_relative_to_game_dir() {
    let (_tmp, path, _) = game_with_file("level0", &["Player"]);
    let data_dir = path.parent().unwrap();

    rabex()
        .arg("--game-dir")
        .arg(data_dir)
        .args(["--file", "level0", "ls"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject"));
}

/// `ls` on a whole game lists its serialized files.
#[test]
fn ls_game_dir_lists_serialized_files() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (ggm, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), ggm).unwrap();
    let (lvl, _) = Flat::new(&["Camera"]).write();
    std::fs::write(data_dir.join("level0"), lvl).unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicates::str::contains("globalgamemanagers"))
        .stdout(predicates::str::contains("level0"));
}

#[test]
fn ls_standalone_bundle() {
    let (_tmp, path, _) = standalone_bundle(&["Player", "Camera"]);

    rabex()
        .arg("--bundle")
        .arg(&path)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2));
}

// -----------------------------------------------------------------------------
// obj
// -----------------------------------------------------------------------------

#[test]
fn obj_dumps_json_to_stdout() {
    let (_tmp, path, go_ids) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("--file")
        .arg(&path)
        .arg("obj")
        .arg(go_ids[0].to_string())
        .assert()
        .success();

    let value = stdout_json(&assert);
    assert_eq!(value["m_Name"], "Player");
}

#[test]
fn obj_from_standalone_bundle() {
    let (_tmp, path, go_ids) = standalone_bundle(&["Player"]);

    let assert = rabex()
        .arg("--bundle")
        .arg(&path)
        .arg("obj")
        .arg(go_ids[0].to_string())
        .assert()
        .success();

    let value = stdout_json(&assert);
    assert_eq!(value["m_Name"], "Player");
}

#[test]
fn obj_missing_id_errors_without_redundancy() {
    let (_tmp, path, _) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("--file")
        .arg(&path)
        .args(["obj", "9999"])
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("9999"), "stderr: {stderr}");
    assert!(
        !stderr.contains("Caused by"),
        "error should not be wrapped redundantly: {stderr}"
    );
}

// -----------------------------------------------------------------------------
// info
// -----------------------------------------------------------------------------

#[test]
fn info_serialized_file_reports_header() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("--file")
        .arg(&path)
        .arg("info")
        .assert()
        .success()
        .stdout(predicates::str::contains("serialized file"))
        .stdout(predicates::str::contains("objects: 4"))
        .stdout(predicates::str::contains(format!(
            "unity version: {}",
            fixtures::TEST_UNITY_VERSION
        )));
}

#[test]
fn info_standalone_bundle_lists_entries() {
    let (_tmp, path, _) = standalone_bundle(&["Player"]);

    rabex()
        .arg("--bundle")
        .arg(&path)
        .arg("info")
        .assert()
        .success()
        .stdout(predicates::str::contains("bundle"))
        .stdout(predicates::str::contains("serialized"))
        .stdout(predicates::str::contains("CAB-test"));
}

#[test]
fn info_game_dir_reports_summary() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (ggm, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), ggm).unwrap();
    let (lvl, _) = Flat::new(&["Camera"]).write();
    std::fs::write(data_dir.join("level0"), lvl).unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .arg("info")
        .assert()
        .success()
        .stdout(predicates::str::contains("game directory"))
        .stdout(predicates::str::contains("serialized files:"))
        .stdout(predicates::str::contains("addressables:"));
}

/// A `--bundle` with the UnityFS magic is routed to the bundle reader; a
/// truncated one fails there rather than being misread.
#[test]
fn info_truncated_bundle_errors() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("x.bundle");
    std::fs::write(&path, b"UnityFS\0\0\0\0\0").unwrap();

    rabex()
        .arg("--bundle")
        .arg(&path)
        .arg("info")
        .assert()
        .failure();
}

// -----------------------------------------------------------------------------
// target resolution
// -----------------------------------------------------------------------------

#[test]
fn no_target_errors() {
    rabex()
        .arg("info")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no target given"));
}

#[test]
fn file_and_bundle_mutually_exclusive() {
    let (_tmp, file, _) = standalone_file(&["Player"]);

    rabex()
        .arg("--file")
        .arg(&file)
        .args(["--bundle", "whatever", "info"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn obj_on_game_dir_bails_cleanly() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (bytes, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), bytes).unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["obj", "1"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "expected a file or bundle, not a game directory",
        ));
}

// -----------------------------------------------------------------------------
// completion
// -----------------------------------------------------------------------------

/// Run the binary as the `COMPLETE=fish` shim does, completing the `obj`
/// path-id slot for `--file <path> obj <current>`:
/// `rabex -- rabex --file <path> obj <current-token>`.
fn complete_obj_path_id(path: &Path, current: &str) -> String {
    let assert = rabex()
        .env("COMPLETE", "fish")
        .arg("--")
        .arg("rabex")
        .arg("--file")
        .arg(path)
        .arg("obj")
        .arg(current)
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// Empty `path_id` token must still offer every id (regression: empty token
/// once returned only flags because arg re-parsing choked on the empty id).
#[test]
fn completion_empty_path_id_offers_all_ids() {
    let (_tmp, path, go_ids) = standalone_file(&["Player", "Camera"]);
    let stdout = complete_obj_path_id(&path, "");

    for id in &go_ids {
        assert!(
            stdout
                .lines()
                .any(|l| l.split('\t').next() == Some(&id.to_string())),
            "completion missing id {id}:\n{stdout}"
        );
    }
    assert!(stdout.contains("GameObject"), "stdout:\n{stdout}");
}

#[test]
fn completion_path_id_prefix_filters() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);
    let stdout = complete_obj_path_id(&path, "1");

    let ids: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.split('\t').next())
        .collect();
    assert!(!ids.is_empty(), "expected some candidates:\n{stdout}");
    assert!(
        ids.iter().all(|id| id.starts_with('1')),
        "all candidates should start with 1: {ids:?}"
    );
}

fn stdout_json(assert: &assert_cmd::assert::Assert) -> serde_json::Value {
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).unwrap()
}
