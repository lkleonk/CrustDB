use crate::engine::{Record, RowId, StoredRow};
use crate::error::{CrustDbError, Result};
use crate::storage::record::{decode_record, encode_record, write_u32, write_u64, Cursor};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const TABLE_MAGIC: &[u8; 8] = b"CRUSTTB1";
const OP_INSERT: u8 = 1;
const OP_DELETE: u8 = 2;
const OP_UPDATE: u8 = 3;

#[derive(Debug)]
pub(crate) struct TableLoad {
    pub(crate) rows: Vec<StoredRow>,
    pub(crate) next_row_id: RowId,
}

pub fn table_path(root: &Path, model_name: &str) -> PathBuf {
    root.join("tables").join(format!("{model_name}.tbl"))
}

pub(crate) fn append_insert(
    root: &Path,
    model_name: &str,
    row_id: RowId,
    record: &Record,
) -> Result<()> {
    append_operation(root, model_name, OP_INSERT, row_id, Some(record))
}

pub(crate) fn append_update(
    root: &Path,
    model_name: &str,
    row_id: RowId,
    record: &Record,
) -> Result<()> {
    append_operation(root, model_name, OP_UPDATE, row_id, Some(record))
}

pub(crate) fn append_delete(root: &Path, model_name: &str, row_id: RowId) -> Result<()> {
    append_operation(root, model_name, OP_DELETE, row_id, None)
}

pub(crate) fn load_rows(root: &Path, model_name: &str) -> Result<TableLoad> {
    let path = table_path(root, model_name);
    if !path.exists() {
        return Ok(TableLoad {
            rows: Vec::new(),
            next_row_id: 1,
        });
    }

    let bytes = fs::read(&path)?;
    if bytes.is_empty() {
        return Ok(TableLoad {
            rows: Vec::new(),
            next_row_id: 1,
        });
    }

    if bytes.starts_with(TABLE_MAGIC) {
        return replay_operations(&bytes[TABLE_MAGIC.len()..]);
    }

    let rows = load_legacy_records(&bytes)?;
    rewrite_as_operation_log(root, model_name, &rows)?;
    let next_row_id = next_row_id(&rows);

    Ok(TableLoad { rows, next_row_id })
}

fn append_operation(
    root: &Path,
    model_name: &str,
    operation: u8,
    row_id: RowId,
    record: Option<&Record>,
) -> Result<()> {
    ensure_operation_file(root, model_name)?;

    let mut out = Vec::new();
    out.push(operation);
    write_u64(&mut out, row_id);

    if let Some(record) = record {
        let payload = encode_record(record)?;
        write_u32(
            &mut out,
            crate::storage::record::usize_to_u32(payload.len(), "record length")?,
        );
        out.extend_from_slice(&payload);
    }

    let mut file = OpenOptions::new()
        .append(true)
        .open(table_path(root, model_name))?;
    file.write_all(&out)?;
    file.flush()?;

    Ok(())
}

fn ensure_operation_file(root: &Path, model_name: &str) -> Result<()> {
    let table_dir = root.join("tables");
    fs::create_dir_all(&table_dir)?;
    let path = table_path(root, model_name);

    if !path.exists() || fs::metadata(&path)?.len() == 0 {
        fs::write(path, TABLE_MAGIC)?;
        return Ok(());
    }

    let bytes = fs::read(&path)?;
    if bytes.starts_with(TABLE_MAGIC) {
        return Ok(());
    }

    Err(CrustDbError::StorageFormat(
        "legacy table file must be loaded before appending operations".to_string(),
    ))
}

fn replay_operations(bytes: &[u8]) -> Result<TableLoad> {
    let mut cursor = Cursor::new(bytes);
    let mut rows = Vec::new();
    let mut row_positions = HashMap::new();
    let mut max_row_id = 0;

    while !cursor.is_finished() {
        let operation = cursor.read_u8()?;
        let row_id = cursor.read_u64()?;
        max_row_id = max_row_id.max(row_id);

        match operation {
            OP_INSERT => {
                let record = read_operation_record(&mut cursor)?;
                if row_positions.contains_key(&row_id) {
                    return Err(CrustDbError::StorageFormat(format!(
                        "duplicate row id in table file: {row_id}"
                    )));
                }
                row_positions.insert(row_id, rows.len());
                rows.push(StoredRow {
                    row_id,
                    record,
                    deleted: false,
                });
            }
            OP_DELETE => {
                let position = live_position(&rows, &row_positions, row_id)?;
                rows[position].deleted = true;
            }
            OP_UPDATE => {
                let record = read_operation_record(&mut cursor)?;
                let position = live_position(&rows, &row_positions, row_id)?;
                rows[position].record = record;
            }
            other => {
                return Err(CrustDbError::StorageFormat(format!(
                    "unknown table operation: {other}"
                )));
            }
        }
    }

    Ok(TableLoad {
        rows,
        next_row_id: max_row_id.saturating_add(1).max(1),
    })
}

fn read_operation_record(cursor: &mut Cursor<'_>) -> Result<Record> {
    let record_len = cursor.read_u32()? as usize;
    let payload = cursor.read_exact(record_len)?;
    decode_record(payload)
}

fn live_position(
    rows: &[StoredRow],
    row_positions: &HashMap<RowId, usize>,
    row_id: RowId,
) -> Result<usize> {
    let Some(position) = row_positions.get(&row_id).copied() else {
        return Err(CrustDbError::StorageFormat(format!(
            "operation references unknown row id: {row_id}"
        )));
    };

    if rows[position].deleted {
        return Err(CrustDbError::StorageFormat(format!(
            "operation references deleted row id: {row_id}"
        )));
    }

    Ok(position)
}

fn load_legacy_records(bytes: &[u8]) -> Result<Vec<StoredRow>> {
    let mut cursor = Cursor::new(bytes);
    let mut rows = Vec::new();

    while !cursor.is_finished() {
        let record_len = cursor.read_u32()? as usize;
        let payload = cursor.read_exact(record_len)?;
        rows.push(StoredRow {
            row_id: rows.len() as RowId + 1,
            record: decode_record(payload)?,
            deleted: false,
        });
    }

    Ok(rows)
}

fn rewrite_as_operation_log(root: &Path, model_name: &str, rows: &[StoredRow]) -> Result<()> {
    let mut out = Vec::new();
    out.extend_from_slice(TABLE_MAGIC);

    for row in rows {
        out.push(OP_INSERT);
        write_u64(&mut out, row.row_id);
        let payload = encode_record(&row.record)?;
        write_u32(
            &mut out,
            crate::storage::record::usize_to_u32(payload.len(), "record length")?,
        );
        out.extend_from_slice(&payload);
    }

    fs::write(table_path(root, model_name), out)?;
    Ok(())
}

fn next_row_id(rows: &[StoredRow]) -> RowId {
    rows.iter()
        .map(|row| row.row_id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crustdb-{label}-{}-{unique}", std::process::id()))
    }

    fn record(id: i64, username: &str) -> Record {
        BTreeMap::from([
            ("id".to_string(), Value::Int(id)),
            ("username".to_string(), Value::String(username.to_string())),
        ])
    }

    #[test]
    fn table_file_replays_insert_update_delete_operations() {
        let root = temp_path("table-operations");

        append_insert(&root, "User", 1, &record(1, "alice")).unwrap();
        append_update(&root, "User", 1, &record(1, "alice2")).unwrap();
        append_insert(&root, "User", 2, &record(2, "bob")).unwrap();
        append_delete(&root, "User", 2).unwrap();

        let loaded = load_rows(&root, "User").unwrap();

        assert_eq!(loaded.next_row_id, 3);
        assert_eq!(loaded.rows.len(), 2);
        assert_eq!(
            loaded.rows[0].record.get("username"),
            Some(&Value::String("alice2".to_string()))
        );
        assert!(!loaded.rows[0].deleted);
        assert!(loaded.rows[1].deleted);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_table_file_is_migrated_to_operation_log() {
        let root = temp_path("table-legacy");
        let path = table_path(&root, "User");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        let payload = encode_record(&record(1, "alice")).unwrap();
        let mut bytes = Vec::new();
        write_u32(&mut bytes, payload.len() as u32);
        bytes.extend_from_slice(&payload);
        fs::write(&path, bytes).unwrap();

        let loaded = load_rows(&root, "User").unwrap();
        let migrated = fs::read(&path).unwrap();

        assert_eq!(loaded.next_row_id, 2);
        assert_eq!(loaded.rows[0].row_id, 1);
        assert!(migrated.starts_with(TABLE_MAGIC));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_table_record_returns_storage_error() {
        let root = temp_path("table-corrupt");
        let path = table_path(&root, "User");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, 99_u32.to_le_bytes()).unwrap();

        let err = load_rows(&root, "User").unwrap_err();

        assert!(err.to_string().contains("unexpected end of record"));
        let _ = fs::remove_dir_all(root);
    }
}
