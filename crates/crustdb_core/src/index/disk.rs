use crate::engine::{RowId, StoredRow};
use crate::error::{CrustDbError, Result};
use crate::index::file::{load_index, lookup_index, save_index};
use crate::index::manifest;
use crate::index::TableIndexes;
use crate::schema::ModelSchema;
use crate::value::Value;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn load_or_rebuild_all(
    root: &Path,
    schemas: &HashMap<String, ModelSchema>,
    rows: &HashMap<String, Vec<StoredRow>>,
) -> Result<HashMap<String, TableIndexes>> {
    if manifest::is_clean(root)? {
        match load_all(root, schemas) {
            Ok(indexes) => return Ok(indexes),
            Err(CrustDbError::StorageFormat(_)) => {}
            Err(error) => return Err(error),
        }
    }

    rebuild_all(root, schemas, rows)
}

pub(crate) fn load_or_rebuild_model(
    root: &Path,
    model_name: &str,
    schema: &ModelSchema,
    rows: &[StoredRow],
) -> Result<TableIndexes> {
    if manifest::is_clean(root)? {
        match load_model(root, model_name, schema) {
            Ok(indexes) => return Ok(indexes),
            Err(CrustDbError::StorageFormat(_)) => {}
            Err(error) => return Err(error),
        }
    }

    let indexes = TableIndexes::build(schema, rows)?;
    manifest::mark_dirty(root)?;
    save_model(root, model_name, schema, &indexes)?;
    manifest::mark_clean(root)?;
    Ok(indexes)
}

pub(crate) fn mark_dirty(root: &Path) -> Result<()> {
    manifest::mark_dirty(root)
}

pub(crate) fn mark_clean(root: &Path) -> Result<()> {
    manifest::mark_clean(root)
}

pub(crate) fn save_model(
    root: &Path,
    model_name: &str,
    schema: &ModelSchema,
    indexes: &TableIndexes,
) -> Result<()> {
    for field_name in TableIndexes::indexed_field_names(schema) {
        let entries = indexes.entries_for_field(field_name);
        save_index(root, model_name, field_name, &entries)?;
    }

    Ok(())
}

pub(crate) fn lookup_exact(
    root: &Path,
    model_name: &str,
    field_name: &str,
    value: &Value,
) -> Result<Option<RowId>> {
    lookup_index(root, model_name, field_name, value)
}

fn load_all(
    root: &Path,
    schemas: &HashMap<String, ModelSchema>,
) -> Result<HashMap<String, TableIndexes>> {
    let mut indexes = HashMap::new();
    for (model_name, schema) in schemas {
        indexes.insert(model_name.clone(), load_model(root, model_name, schema)?);
    }
    Ok(indexes)
}

fn load_model(root: &Path, model_name: &str, schema: &ModelSchema) -> Result<TableIndexes> {
    let mut entries_by_field = HashMap::new();
    for field_name in TableIndexes::indexed_field_names(schema) {
        entries_by_field.insert(
            field_name.to_string(),
            load_index(root, model_name, field_name)?,
        );
    }

    TableIndexes::from_entries(schema, entries_by_field)
}

fn rebuild_all(
    root: &Path,
    schemas: &HashMap<String, ModelSchema>,
    rows: &HashMap<String, Vec<StoredRow>>,
) -> Result<HashMap<String, TableIndexes>> {
    manifest::mark_dirty(root)?;

    let mut indexes = HashMap::new();
    for (model_name, schema) in schemas {
        let model_rows = rows.get(model_name).map(Vec::as_slice).unwrap_or(&[]);
        let table_indexes = TableIndexes::build(schema, model_rows)?;
        save_model(root, model_name, schema, &table_indexes)?;
        indexes.insert(model_name.clone(), table_indexes);
    }

    manifest::mark_clean(root)?;
    Ok(indexes)
}
