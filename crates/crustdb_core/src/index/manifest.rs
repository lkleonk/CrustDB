use crate::error::Result;
use std::fs;
use std::path::Path;

const INDEX_MANIFEST_MAGIC: &[u8; 8] = b"CRUSTIXM";
const INDEX_MANIFEST_VERSION: u8 = 1;
const CLEAN: u8 = 0;
const DIRTY: u8 = 1;

pub(crate) fn is_clean(root: &Path) -> Result<bool> {
    let path = manifest_path(root);
    if !path.exists() {
        return Ok(false);
    }

    let bytes = fs::read(path)?;
    if bytes.len() != INDEX_MANIFEST_MAGIC.len() + 2 {
        return Ok(false);
    }

    if &bytes[..INDEX_MANIFEST_MAGIC.len()] != INDEX_MANIFEST_MAGIC {
        return Ok(false);
    }

    if bytes[INDEX_MANIFEST_MAGIC.len()] != INDEX_MANIFEST_VERSION {
        return Ok(false);
    }

    Ok(bytes[INDEX_MANIFEST_MAGIC.len() + 1] == CLEAN)
}

pub(crate) fn mark_dirty(root: &Path) -> Result<()> {
    write_state(root, DIRTY)
}

pub(crate) fn mark_clean(root: &Path) -> Result<()> {
    write_state(root, CLEAN)
}

fn write_state(root: &Path, state: u8) -> Result<()> {
    let index_root = root.join("indexes");
    fs::create_dir_all(&index_root)?;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(INDEX_MANIFEST_MAGIC);
    bytes.push(INDEX_MANIFEST_VERSION);
    bytes.push(state);
    fs::write(manifest_path(root), bytes)?;

    Ok(())
}

fn manifest_path(root: &Path) -> std::path::PathBuf {
    root.join("indexes").join("manifest.crustix")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crustdb-{label}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn missing_manifest_is_not_clean() {
        let root = temp_path("missing-index-manifest");

        assert!(!is_clean(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_tracks_dirty_and_clean_state() {
        let root = temp_path("index-manifest");

        mark_dirty(&root).unwrap();
        assert!(!is_clean(&root).unwrap());

        mark_clean(&root).unwrap();
        assert!(is_clean(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }
}
