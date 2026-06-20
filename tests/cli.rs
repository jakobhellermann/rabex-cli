//! CLI-level tests — invoke the `rabex` binary and assert on stdout/stderr and
//! exit codes. Fixtures are written into ephemeral temp dirs; for game-context
//! tests the layout mimics a Unity game (`*_Data/`). Nothing binary is
//! committed; the bytes are synthesized per test.

mod fixtures;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use fixtures::Flat;
use tempfile::TempDir;

/// The CAB name used by [`standalone_bundle`].
const BUNDLE_CAB: &str = "CAB-test";

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
    let bytes = fixtures::bundle_with_serialized(BUNDLE_CAB, &serialized);
    let path = tmp.path().join("loose.bundle");
    std::fs::write(&path, bytes).unwrap();
    (tmp, path, go_ids)
}

fn rabex() -> Command {
    Command::cargo_bin("rabex").unwrap()
}

fn stdout_json(assert: &assert_cmd::assert::Assert) -> serde_json::Value {
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).unwrap()
}

// -----------------------------------------------------------------------------
// file <path> objects / object
// -----------------------------------------------------------------------------

#[test]
fn file_objects_lists_objects() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("file")
        .arg(&path)
        .arg("objects")
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2))
        .stdout(predicates::str::contains("Transform").count(2));
}

/// `objects` shows each object's `m_Name` by default (no `--names` flag).
#[test]
fn file_objects_shows_names_by_default() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("file")
        .arg(&path)
        .arg("objects")
        .assert()
        .success()
        .stdout(predicates::str::contains("Player"))
        .stdout(predicates::str::contains("Camera"));
}

/// `objects --type <script>` matches MonoBehaviours by script class name and the
/// listing shows the script as `(Script)`.
#[test]
fn file_objects_type_matches_monobehaviour_script() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (bytes, _go, _mb) = fixtures::scene_with_script_component("Player", "PlayMakerFSM");
    std::fs::write(data_dir.join("level0"), bytes).unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["file", "level0", "objects", "--type", "PlayMakerFSM"])
        .assert()
        .success()
        .stdout(predicates::str::contains("MonoBehaviour"))
        .stdout(predicates::str::contains("(PlayMakerFSM)"));
}

#[test]
fn file_objects_type_filter() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("file")
        .arg(&path)
        .args(["objects", "--type", "GameObject"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2))
        .stdout(predicates::str::contains("Transform").count(0));
}

/// `--game-dir DIR file NAME objects` resolves the file relative to the game.
#[test]
fn file_objects_relative_to_game_dir() {
    let (_tmp, path, _) = game_with_file("level0", &["Player"]);
    let data_dir = path.parent().unwrap();

    rabex()
        .arg("--game-dir")
        .arg(data_dir)
        .args(["file", "level0", "objects"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject"));
}

#[test]
fn file_object_cat_dumps_json() {
    let (_tmp, path, go_ids) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("file")
        .arg(&path)
        .args(["object", &go_ids[0].to_string(), "cat"])
        .assert()
        .success();

    let value = stdout_json(&assert);
    assert_eq!(value["m_Name"], "Player");
}

/// A negative path id (common in real bundles) must be accepted as the value,
/// not rejected as an unknown flag. It fails later as "no such object".
#[test]
fn file_object_accepts_negative_path_id() {
    let (_tmp, path, _) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("file")
        .arg(&path)
        .args(["object", "-8333449340390664235", "cat"])
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        !stderr.contains("unexpected argument"),
        "negative id should parse as a value, not a flag: {stderr}"
    );
    assert!(
        stderr.contains("8333449340390664235"),
        "should fail looking up the id: {stderr}"
    );
}

#[test]
fn file_object_missing_id_errors_without_redundancy() {
    let (_tmp, path, _) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("file")
        .arg(&path)
        .args(["object", "9999", "cat"])
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("9999"), "stderr: {stderr}");
    assert!(
        !stderr.contains("Caused by"),
        "error should not be wrapped redundantly: {stderr}"
    );
}

/// `object <component-path> cat` resolves by hierarchy path and dumps it.
#[test]
fn file_object_cat_by_component_path() {
    let (_tmp, path, _) = standalone_file(&["Player"]);

    let assert = rabex()
        .arg("file")
        .arg(&path)
        .args(["object", "Player@Transform", "cat"])
        .assert()
        .success();

    let value = stdout_json(&assert);
    assert!(value.get("m_GameObject").is_some(), "{value}");
}

/// `object <id> references` names the target by `m_Name` and labels each
/// referrer with its class and the GameObject it sits on (here the GameObject's
/// own Transform points back at it).
#[test]
fn file_object_references_resolves_names() {
    let (_tmp, path, go_ids) = game_with_file("level0", &["Player"]);
    let data_dir = path.parent().unwrap();

    rabex()
        .arg("--game-dir")
        .arg(data_dir)
        .args([
            "file",
            "level0",
            "object",
            &go_ids[0].to_string(),
            "references",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("1 reference(s) to Player:"))
        .stdout(predicates::str::contains("Transform (on 'Player')"));
}

/// `object <id> references` hides preload-table referrers (`PreloadData` /
/// `AssetBundle`) by default; `--include-preloads` brings them back.
#[test]
fn file_object_references_filters_preloads() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (bytes, go_id) = fixtures::scene_with_preload("Player");
    std::fs::write(data_dir.join("level0"), bytes).unwrap();

    // Default: only the Transform (a real user); the PreloadData is hidden.
    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["file", "level0", "object", &go_id.to_string(), "references"])
        .assert()
        .success()
        .stdout(predicates::str::contains("1 reference(s) to Player:"))
        .stdout(predicates::str::contains("Transform (on 'Player')"))
        .stdout(predicates::str::contains("PreloadData").count(0));

    // With --include-preloads: the PreloadData referrer is listed too.
    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args([
            "file",
            "level0",
            "object",
            &go_id.to_string(),
            "references",
            "--include-preloads",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("2 reference(s) to Player:"))
        .stdout(predicates::str::contains("PreloadData"));
}

/// `file <path> find <TYPE>` lists the GameObject(s) carrying that component,
/// with a re-usable component path.
#[test]
fn file_find_lists_component_holders() {
    let (_tmp, path, _) = game_with_file("level0", &["Player"]);
    let data_dir = path.parent().unwrap();

    rabex()
        .arg("--game-dir")
        .arg(data_dir)
        .args(["file", "level0", "find", "Transform"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Player@Transform"));
}

/// A malformed component path is rejected at parse time, before doing any work.
#[test]
fn file_object_rejects_bad_index() {
    let (_tmp, path, _) = standalone_file(&["Player"]);

    rabex()
        .arg("file")
        .arg(&path)
        .args(["object", "Player:x", "cat"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("invalid index"));
}

#[test]
fn file_info_reports_header() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);

    rabex()
        .arg("file")
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

// -----------------------------------------------------------------------------
// collections: files / game
// -----------------------------------------------------------------------------

/// `files` lists the game's serialized files.
#[test]
fn files_lists_serialized_files() {
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
        .arg("files")
        .assert()
        .success()
        .stdout(predicates::str::contains("globalgamemanagers"))
        .stdout(predicates::str::contains("level0"));
}

#[test]
fn game_script_locations_maps_scripts_to_files() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("level0"),
        fixtures::scripts_file(&["HeroController", "EnemyController"]),
    )
    .unwrap();

    let assert = rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["--format", "json", "game", "script-locations"])
        .assert()
        .success();

    // Sorted by script name; each maps to the file it lives in.
    assert_eq!(
        stdout_json(&assert),
        serde_json::json!([
            { "script": "EnemyController", "locations": ["level0"] },
            { "script": "HeroController", "locations": ["level0"] },
        ])
    );
}

#[test]
fn game_script_locations_filter_narrows_by_name() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("level0"),
        fixtures::scripts_file(&["HeroController", "EnemyController"]),
    )
    .unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["game", "script-locations", "hero"])
        .assert()
        .success()
        .stdout("HeroController\n  level0\n");
}

#[test]
fn game_info_reports_summary() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (ggm, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), ggm).unwrap();

    rabex()
        .arg("--game-dir")
        .arg(&data_dir)
        .args(["game", "info"])
        .assert()
        .success()
        .stdout(predicates::str::contains("game directory"))
        .stdout(predicates::str::contains("serialized files:"))
        .stdout(predicates::str::contains("addressables:"));
}

/// With no `--game-dir`/`--steam-game`, the game is detected from the cwd.
#[test]
fn game_detected_from_current_dir() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("Game_Data");
    std::fs::create_dir(&data_dir).unwrap();
    let (ggm, _) = Flat::new(&["Player"]).write();
    std::fs::write(data_dir.join("globalgamemanagers"), ggm).unwrap();

    rabex()
        .current_dir(tmp.path())
        .args(["game", "info"])
        .assert()
        .success()
        .stdout(predicates::str::contains("game directory"));
}

// -----------------------------------------------------------------------------
// bundle
// -----------------------------------------------------------------------------

/// `bundle <path> files` lists the bundle's contained files (CABs).
#[test]
fn bundle_files_lists_files() {
    let (_tmp, path, _) = standalone_bundle(&["Player", "Camera"]);

    rabex()
        .arg("bundle")
        .arg(&path)
        .arg("files")
        .assert()
        .success()
        .stdout(predicates::str::contains(BUNDLE_CAB));
}

/// `bundle <path> info` lists the bundle's entries with sizes and kinds.
#[test]
fn bundle_info_lists_entries() {
    let (_tmp, path, _) = standalone_bundle(&["Player"]);

    rabex()
        .arg("bundle")
        .arg(&path)
        .arg("info")
        .assert()
        .success()
        .stdout(predicates::str::contains("bundle"))
        .stdout(predicates::str::contains("serialized"))
        .stdout(predicates::str::contains(BUNDLE_CAB));
}

/// `bundle <path> file <cab> objects` lists the objects in a contained file.
#[test]
fn bundle_file_objects_lists_objects() {
    let (_tmp, path, _) = standalone_bundle(&["Player", "Camera"]);

    rabex()
        .arg("bundle")
        .arg(&path)
        .args(["file", BUNDLE_CAB, "objects"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2));
}

/// `bundle <path> file <cab> object <id> cat` dumps an object from a CAB.
#[test]
fn bundle_file_object_cat_dumps_json() {
    let (_tmp, path, go_ids) = standalone_bundle(&["Player"]);

    let assert = rabex()
        .arg("bundle")
        .arg(&path)
        .args(["file", BUNDLE_CAB, "object", &go_ids[0].to_string(), "cat"])
        .assert()
        .success();

    let value = stdout_json(&assert);
    assert_eq!(value["m_Name"], "Player");
}

/// `bundle <path> file objects` without a CAB defaults to the bundle's main
/// serialized file (and the trailing verb is not mistaken for the CAB).
#[test]
fn bundle_file_default_cab_lists_objects() {
    let (_tmp, path, _) = standalone_bundle(&["Player", "Camera"]);

    rabex()
        .arg("bundle")
        .arg(&path)
        .args(["file", "objects"])
        .assert()
        .success()
        .stdout(predicates::str::contains("GameObject").count(2));
}

/// `bundle <path> file object <name>` selects an object by its `m_Name` — here a
/// `MonoScript` in the bundle's main file, with the CAB left to default.
#[test]
fn bundle_file_object_by_name() {
    let tmp = TempDir::new().unwrap();
    let serialized = fixtures::scripts_file(&["PlayMakerFSM", "FsmTemplate"]);
    let bytes = fixtures::bundle_with_serialized(BUNDLE_CAB, &serialized);
    let path = tmp.path().join("monoscripts.bundle");
    std::fs::write(&path, bytes).unwrap();

    rabex()
        .arg("bundle")
        .arg(&path)
        .args(["file", "object", "PlayMakerFSM", "info"])
        .assert()
        .success()
        .stdout(predicates::str::contains("MonoScript"))
        .stdout(predicates::str::contains("PlayMakerFSM"));
}

/// `bundle <path> file object <TAB>` completes object refs even with the CAB
/// left to default (the verb operates on the bundle's main serialized file).
#[test]
fn bundle_file_object_completes_with_default_cab() {
    let tmp = TempDir::new().unwrap();
    let serialized = fixtures::scripts_file(&["PlayMakerFSM", "FsmTemplate"]);
    let bytes = fixtures::bundle_with_serialized(BUNDLE_CAB, &serialized);
    let path = tmp.path().join("monoscripts.bundle");
    std::fs::write(&path, bytes).unwrap();

    rabex()
        .env("COMPLETE", "fish")
        .args(["--", "rabex", "bundle"])
        .arg(&path)
        .args(["file", "object", ""])
        .assert()
        .success()
        .stdout(predicates::str::contains("PlayMakerFSM"));
}

/// A truncated bundle fails in the bundle reader rather than being misread.
#[test]
fn bundle_truncated_errors() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("x.bundle");
    std::fs::write(&path, b"UnityFS\0\0\0\0\0").unwrap();

    rabex()
        .arg("bundle")
        .arg(&path)
        .arg("info")
        .assert()
        .failure();
}

// -----------------------------------------------------------------------------
// context resolution
// -----------------------------------------------------------------------------

/// A game command without a game context errors clearly.
#[test]
fn game_command_without_context_errors() {
    rabex()
        .args(["game", "info"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no game"));
}

/// `bundles` (list all) needs a game context.
#[test]
fn bundles_without_context_needs_game() {
    rabex()
        .arg("bundles")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no game"));
}

// -----------------------------------------------------------------------------
// completion
// -----------------------------------------------------------------------------

/// Run the binary as the `COMPLETE=fish` shim does, completing the `object`
/// ref slot for `file <path> object <current>`.
fn complete_object_ref(path: &Path, current: &str) -> String {
    let assert = rabex()
        .env("COMPLETE", "fish")
        .arg("--")
        .arg("rabex")
        .arg("file")
        .arg(path)
        .arg("object")
        .arg(current)
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// Empty ref token must offer every path id (regression: empty token once
/// returned only flags because arg re-parsing choked on the empty id).
#[test]
fn completion_empty_object_ref_offers_all_ids() {
    let (_tmp, path, go_ids) = standalone_file(&["Player", "Camera"]);
    let stdout = complete_object_ref(&path, "");

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
fn completion_object_ref_prefix_filters() {
    let (_tmp, path, _) = standalone_file(&["Player", "Camera"]);
    let stdout = complete_object_ref(&path, "1");

    let ids: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter(|t| t.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .collect();
    assert!(!ids.is_empty(), "expected some candidates:\n{stdout}");
    assert!(
        ids.iter().all(|id| id.starts_with('1')),
        "all numeric candidates should start with 1: {ids:?}"
    );
}
