use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// What a given path actually is, decided by inspecting it.
#[derive(Debug)]
pub enum Target {
    /// A standalone Unity serialized file (globalgamemanagers, level0, sharedassets, …).
    SerializedFile(PathBuf),
    /// A UnityFS asset bundle (possibly containing several serialized files).
    Bundle(PathBuf),
    /// A game directory (`*_Data`, or a dir we can `GameFiles::probe`).
    GameDir(PathBuf),
}

impl Target {
    /// Auto-detect what `path` is, by metadata + magic bytes.
    pub fn detect(path: &Path) -> Result<Target> {
        let meta =
            std::fs::metadata(path).with_context(|| format!("cannot stat {}", path.display()))?;

        if meta.is_dir() {
            // TODO: distinguish a real game dir from "a dir full of bundles".
            return Ok(Target::GameDir(path.to_path_buf()));
        }

        if is_bundle(path)? {
            Ok(Target::Bundle(path.to_path_buf()))
        } else {
            // TODO: could still verify it parses as a SerializedFile and error early otherwise.
            Ok(Target::SerializedFile(path.to_path_buf()))
        }
    }
}

/// UnityFS bundles start with the ASCII magic `UnityFS`.
fn is_bundle(path: &Path) -> Result<bool> {
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 7];
    match f.read_exact(&mut magic) {
        Ok(()) => Ok(looks_like_bundle(&magic)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Whether `header` (the first bytes of a file) is a UnityFS bundle magic.
fn looks_like_bundle(header: &[u8]) -> bool {
    header.starts_with(b"UnityFS")
}

#[cfg(test)]
mod tests {
    use super::looks_like_bundle;

    #[test]
    fn bundle_magic_detection() {
        assert!(looks_like_bundle(b"UnityFS\0\0\0"));
        assert!(!looks_like_bundle(b"\x00\x00\x00\x00not a bundle"));
        // A serialized file does not start with the bundle magic.
        assert!(!looks_like_bundle(b"Unity"));
        assert!(!looks_like_bundle(b""));
    }
}
