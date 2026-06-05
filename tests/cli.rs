//! CLI-level tests — invoke the `rabex` binary and assert on stdout/stderr and
//! exit codes. Fixtures are written into an ephemeral temp dir laid out like a
//! Unity game (`*_Data/`), so the auto-detect + game-dir walk-up paths run for
//! real. Nothing binary is committed; the bytes are synthesized per test.

mod fixtures;

use std::path::PathBuf;

use assert_cmd::Command;
use fixtures::Flat;
use tempfile::TempDir;

/// Write a flat scene into `<tmp>/Game_Data/<file>` and return (tempdir, path).
/// The tempdir guard must stay alive for the file to exist.
fn game_with_file(file: &str, names: &[&'static str]) -> (TempDir, PathBuf, Vec<i64>) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();

    let (bytes, go_ids) = Flat::new(names).write();
    let path = data_dir.join(file);
    std::fs::write(&path, bytes).unwrap();

    (tmp, path, go_ids)
}

fn rabex() -> Command {
    Command::cargo_bin("rabex").unwrap()
}

#[test]
fn ls_serialized_file_in_game_dir() {
    let (_tmp, path, _) = game_with_file("level0", &["Player", "Camera"]);

    rabex()
        .arg("ls")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2))
        .stdout(predicates::str::contains("Transform").count(2));
}

#[test]
fn obj_dumps_json_to_stdout() {
    let (_tmp, path, go_ids) = game_with_file("level0", &["Player"]);

    let assert = rabex()
        .arg("obj")
        .arg(&path)
        .arg(go_ids[0].to_string())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["m_Name"], "Player");
}

#[test]
fn obj_missing_id_errors_without_redundancy() {
    let (_tmp, path, _) = game_with_file("level0", &["Player"]);

    let assert = rabex().arg("obj").arg(&path).arg("9999").assert().failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    // Exactly one mention of the id — no "Caused by:" duplicate of the same text.
    assert!(stderr.contains("9999"), "stderr: {stderr}");
    assert!(
        !stderr.contains("Caused by"),
        "error should not be wrapped redundantly: {stderr}"
    );
}

#[test]
fn nonexistent_path_errors() {
    rabex()
        .arg("info")
        .arg("/no/such/path/whatsoever")
        .assert()
        .failure();
}

#[test]
fn info_serialized_file_reports_header() {
    let (_tmp, path, _) = game_with_file("level0", &["Player", "Camera"]);

    rabex()
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("serialized file"))
        // Four objects: two GameObjects + two Transforms.
        .stdout(predicates::str::contains("objects: 4"))
        .stdout(predicates::str::contains(format!(
            "unity version: {}",
            fixtures::TEST_UNITY_VERSION
        )));
}

/// A file with the UnityFS magic is detected as a bundle and routed to the
/// bundle reader; a truncated one fails there rather than being misread as a
/// serialized file. (Synthesizing a valid bundle is left to end-to-end runs;
/// here we only pin the detect → bundle-reader path and its error handling.)
#[test]
fn info_truncated_bundle_errors_as_bundle() {
    let (_tmp, path, _) = game_with_file("x.bundle", &["Player"]);
    // Overwrite with just the magic so detect classifies it as a bundle, but
    // the reader has nothing valid to parse.
    std::fs::write(&path, b"UnityFS\0\0\0\0\0").unwrap();

    rabex().arg("info").arg(&path).assert().failure();
}

#[test]
fn info_game_dir_reports_summary() {
    // A game dir needs a globalgamemanagers to resolve its unity version.
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (ggm, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), ggm).unwrap();
    let (lvl, _) = Flat::new(&["Camera"]).write();
    std::fs::write(data_dir.join("level0"), lvl).unwrap();

    rabex()
        .arg("info")
        .arg(&data_dir)
        .assert()
        .success()
        .stdout(predicates::str::contains("game directory"))
        .stdout(predicates::str::contains("serialized files:"))
        .stdout(predicates::str::contains("addressables:"));
}

/// The game-dir walk-up: a file nested below `*_Data` still finds its game
/// root by climbing ancestors, so `ls` works without the file sitting directly
/// in `*_Data`.
#[test]
fn ls_finds_game_dir_from_nested_file() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("Game_Data").join("Resources");
    std::fs::create_dir_all(&nested).unwrap();
    let (bytes, _) = Flat::new(&["Player"]).write();
    let path = nested.join("level0");
    std::fs::write(&path, bytes).unwrap();

    rabex()
        .arg("ls")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject"));
}

/// A serialized file with no `*_Data` directory above it is still inspectable:
/// the env falls back to a bare resolver rooted at the file's own directory,
/// so `ls` works without any surrounding game.
#[test]
fn ls_standalone_file_without_game_dir() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("plain");
    std::fs::create_dir(&dir).unwrap();
    let (bytes, _) = Flat::new(&["Player"]).write();
    let path = dir.join("level0");
    std::fs::write(&path, bytes).unwrap();

    rabex()
        .arg("ls")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject"));
}

/// Write a standalone `.bundle` (no surrounding game) into a temp dir and
/// return (tempdir, path, gameobject ids inside).
fn standalone_bundle(names: &[&'static str]) -> (TempDir, PathBuf, Vec<i64>) {
    let tmp = TempDir::new().unwrap();
    let (serialized, go_ids) = Flat::new(names).write();
    let bytes = fixtures::bundle_with_serialized("CAB-test", &serialized);
    let path = tmp.path().join("loose.bundle");
    std::fs::write(&path, bytes).unwrap();
    (tmp, path, go_ids)
}

#[test]
fn info_standalone_bundle_lists_entries() {
    let (_tmp, path, _) = standalone_bundle(&["Player"]);

    rabex()
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("bundle"))
        .stdout(predicates::str::contains("serialized"))
        .stdout(predicates::str::contains("CAB-test"));
}

#[test]
fn ls_standalone_bundle() {
    let (_tmp, path, _) = standalone_bundle(&["Player", "Camera"]);

    rabex()
        .arg("ls")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2));
}

#[test]
fn obj_from_standalone_bundle() {
    let (_tmp, path, go_ids) = standalone_bundle(&["Player"]);

    let assert = rabex()
        .arg("obj")
        .arg(&path)
        .arg(go_ids[0].to_string())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["m_Name"], "Player");
}

/// `ls`/`obj` on a game *directory* must bail cleanly, not panic. (Regression:
/// an earlier refactor unwrapped the relative path before checking the target
/// kind, panicking on a game-dir target.)
#[test]
fn ls_on_game_dir_bails_cleanly() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    // A globalgamemanagers makes the dir probe as a real unity game.
    let (bytes, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), bytes).unwrap();

    rabex()
        .arg("ls")
        .arg(&data_dir)
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "expected a file or bundle, not a game directory",
        ));
}

/// Dynamic completion of `path_id`: the `COMPLETE=fish` shim runs the binary as
/// `rabex -- rabex obj <path> <token>`. With an empty token the completer must
/// still offer every path id (the regression we hit: empty token returned only
/// flags because arg re-parsing choked on the empty `path_id`).
#[test]
fn completion_empty_path_id_offers_all_ids() {
    let (_tmp, path, go_ids) = game_with_file("level0", &["Player", "Camera"]);

    let assert = rabex()
        .env("COMPLETE", "fish")
        .arg("--")
        .arg("rabex")
        .arg("obj")
        .arg(&path)
        .arg("") // empty current token → at the path_id slot, nothing typed
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Every GameObject id (and its Transform) should be offered.
    for id in &go_ids {
        assert!(
            stdout
                .lines()
                .any(|l| l.split('\t').next() == Some(&id.to_string())),
            "completion missing id {id}:\n{stdout}"
        );
    }
    // Sanity: candidates carry a class-name help column, not just flags.
    assert!(stdout.contains("GameObject"), "stdout:\n{stdout}");
}

/// A typed prefix filters the offered ids.
#[test]
fn completion_path_id_prefix_filters() {
    let (_tmp, path, _) = game_with_file("level0", &["Player", "Camera"]);

    let assert = rabex()
        .env("COMPLETE", "fish")
        .arg("--")
        .arg("rabex")
        .arg("obj")
        .arg(&path)
        .arg("1")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Only ids starting with "1" — and at least one exists (id 1).
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
