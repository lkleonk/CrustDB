use crate::error::{CrustDbError, Result};
use std::fs;
use std::path::Path;

const MANIFEST_MAGIC: &[u8; 8] = b"CRUSTMF1";
const FORMAT_VERSION: u32 = 1;

pub fn load_or_create(root: &Path) -> Result<()> {
    let path = root.join("manifest.crust");
    if !path.exists() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MANIFEST_MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        fs::write(path, bytes)?;
        return Ok(());
    }

    let bytes = fs::read(path)?;
    if bytes.len() != 12 {
        return Err(CrustDbError::StorageFormat(
            "invalid manifest length".to_string(),
        ));
    }

    if &bytes[..8] != MANIFEST_MAGIC {
        return Err(CrustDbError::StorageFormat(
            "invalid manifest magic".to_string(),
        ));
    }

    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != FORMAT_VERSION {
        return Err(CrustDbError::StorageFormat(format!(
            "unsupported storage format version: {version}"
        )));
    }

    Ok(())
}
