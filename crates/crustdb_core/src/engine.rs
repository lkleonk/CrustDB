use crate::error::{CrustDbError, Result};
use crate::index::{disk as disk_index, TableIndexes};
use crate::schema::{DataType, ModelSchema};
use crate::storage::{manifest, schema_file, table_file};
use crate::value::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

pub type Record = BTreeMap<String, Value>;
pub type RowId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredRow {
    pub(crate) row_id: RowId,
    pub(crate) record: Record,
    pub(crate) deleted: bool,
}

#[derive(Debug)]
pub struct Engine {
    path: PathBuf,
    schemas: HashMap<String, ModelSchema>,
    rows: HashMap<String, Vec<StoredRow>>,
    indexes: HashMap<String, TableIndexes>,
    next_row_ids: HashMap<String, RowId>,
}

impl Engine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        fs::create_dir_all(&path)?;
        manifest::load_or_create(&path)?;

        let schemas = schema_file::load_schemas(&path)?;
        let mut rows = HashMap::new();
        let mut next_row_ids = HashMap::new();

        for model_name in schemas.keys() {
            let loaded = table_file::load_rows(&path, model_name)?;
            next_row_ids.insert(model_name.clone(), loaded.next_row_id);
            rows.insert(model_name.clone(), loaded.rows);
        }
        let indexes = disk_index::load_or_rebuild_all(&path, &schemas, &rows)?;

        Ok(Self {
            path,
            schemas,
            rows,
            indexes,
            next_row_ids,
        })
    }

    pub fn register_schema(&mut self, schema: ModelSchema) -> Result<()> {
        let model_name = schema.name.clone();

        if let Some(existing) = self.schemas.get(&model_name) {
            if existing != &schema {
                return Err(CrustDbError::IncompatibleSchema(format!(
                    "Schema for {model_name} is incompatible with persisted schema"
                )));
            }
        } else {
            self.schemas.insert(model_name.clone(), schema);
            schema_file::save_schemas(&self.path, &self.schemas)?;
        }

        if !self.rows.contains_key(&model_name) {
            let loaded = table_file::load_rows(&self.path, &model_name)?;
            self.next_row_ids
                .insert(model_name.clone(), loaded.next_row_id);
            self.rows.insert(model_name.clone(), loaded.rows);
        }

        self.ensure_next_row_id(&model_name);
        let schema = self.schema(&model_name)?.clone();
        let model_rows = self.rows.get(&model_name).map(Vec::as_slice).unwrap_or(&[]);
        let indexes =
            disk_index::load_or_rebuild_model(&self.path, &model_name, &schema, model_rows)?;
        self.indexes.insert(model_name.clone(), indexes);

        Ok(())
    }

    pub fn insert(&mut self, model_name: &str, values: Record) -> Result<Record> {
        let row = self.prepare_row(model_name, values)?;
        if !self.indexes.contains_key(model_name) {
            self.rebuild_index(model_name)?;
        }
        self.validate_unique(model_name, &row, None)?;

        let row_id = self.next_row_id(model_name);
        disk_index::mark_dirty(&self.path)?;
        table_file::append_insert(&self.path, model_name, row_id, &row)?;

        self.rows
            .entry(model_name.to_string())
            .or_default()
            .push(StoredRow {
                row_id,
                record: row.clone(),
                deleted: false,
            });
        self.next_row_ids
            .insert(model_name.to_string(), row_id.saturating_add(1).max(1));

        let schema = self.schema(model_name)?.clone();
        self.indexes
            .get_mut(model_name)
            .expect("index exists after rebuild")
            .insert(&schema, row_id, &row);
        disk_index::save_model(
            &self.path,
            model_name,
            &schema,
            self.indexes
                .get(model_name)
                .expect("index exists after insert update"),
        )?;
        disk_index::mark_clean(&self.path)?;

        Ok(row)
    }

    pub fn find(&self, model_name: &str, filters: &Record) -> Result<Option<Record>> {
        let Some(row_id) = self.find_row_id(model_name, filters)? else {
            return Ok(None);
        };

        Ok(self
            .live_row_by_id(model_name, row_id)
            .map(|row| row.record.clone()))
    }

    pub fn delete(&mut self, model_name: &str, filters: &Record) -> Result<bool> {
        let Some(row_id) = self.find_row_id(model_name, filters)? else {
            return Ok(false);
        };

        let schema = self.schema(model_name)?.clone();
        let old_record = self
            .live_row_by_id(model_name, row_id)
            .expect("find_row_id returned a live row")
            .record
            .clone();

        disk_index::mark_dirty(&self.path)?;
        table_file::append_delete(&self.path, model_name, row_id)?;

        self.live_row_by_id_mut(model_name, row_id)
            .expect("find_row_id returned a live row")
            .deleted = true;

        if let Some(indexes) = self.indexes.get_mut(model_name) {
            indexes.remove(&schema, row_id, &old_record);
        }
        if let Some(indexes) = self.indexes.get(model_name) {
            disk_index::save_model(&self.path, model_name, &schema, indexes)?;
        }
        disk_index::mark_clean(&self.path)?;

        Ok(true)
    }

    pub fn update(
        &mut self,
        model_name: &str,
        filters: &Record,
        values: Record,
    ) -> Result<Option<Record>> {
        if values.is_empty() {
            return Err(CrustDbError::Validation(
                "update requires at least one value".to_string(),
            ));
        }

        let Some(row_id) = self.find_row_id(model_name, filters)? else {
            return Ok(None);
        };

        let old_record = self
            .live_row_by_id(model_name, row_id)
            .expect("find_row_id returned a live row")
            .record
            .clone();
        let updated_row = self.prepare_update_row(model_name, &old_record, values)?;

        if !self.indexes.contains_key(model_name) {
            self.rebuild_index(model_name)?;
        }
        self.validate_unique(model_name, &updated_row, Some(row_id))?;

        disk_index::mark_dirty(&self.path)?;
        table_file::append_update(&self.path, model_name, row_id, &updated_row)?;

        let schema = self.schema(model_name)?.clone();
        if let Some(indexes) = self.indexes.get_mut(model_name) {
            indexes.remove(&schema, row_id, &old_record);
        }

        self.live_row_by_id_mut(model_name, row_id)
            .expect("find_row_id returned a live row")
            .record = updated_row.clone();

        self.indexes
            .get_mut(model_name)
            .expect("index exists after rebuild")
            .insert(&schema, row_id, &updated_row);
        disk_index::save_model(
            &self.path,
            model_name,
            &schema,
            self.indexes
                .get(model_name)
                .expect("index exists after update"),
        )?;
        disk_index::mark_clean(&self.path)?;

        Ok(Some(updated_row))
    }

    fn find_row_id(&self, model_name: &str, filters: &Record) -> Result<Option<RowId>> {
        self.validate_filters(model_name, filters, "find")?;

        let Some(rows) = self.rows.get(model_name) else {
            return Ok(None);
        };

        if filters.len() == 1 {
            let (field_name, value) = filters.iter().next().expect("filter length checked");
            if let Some(indexes) = self.indexes.get(model_name) {
                if indexes.indexes_field(field_name) {
                    let Some(row_id) =
                        disk_index::lookup_exact(&self.path, model_name, field_name, value)?
                    else {
                        return Ok(None);
                    };
                    if self.live_row_by_id(model_name, row_id).is_some() {
                        return Ok(Some(row_id));
                    }
                    return Ok(None);
                }
            }
        }

        Ok(rows
            .iter()
            .find(|row| {
                !row.deleted
                    && filters
                        .iter()
                        .all(|(field_name, value)| row.record.get(field_name) == Some(value))
            })
            .map(|row| row.row_id))
    }

    fn prepare_row(&self, model_name: &str, values: Record) -> Result<Record> {
        let schema = self.schema(model_name)?;

        let unknown = unknown_fields(values.keys(), schema);
        if !unknown.is_empty() {
            return Err(CrustDbError::Validation(format!(
                "Unknown field(s): {}",
                unknown.join(", ")
            )));
        }

        self.normalize_row(schema, &values)
    }

    fn prepare_update_row(
        &self,
        model_name: &str,
        existing: &Record,
        values: Record,
    ) -> Result<Record> {
        let schema = self.schema(model_name)?;

        let unknown = unknown_fields(values.keys(), schema);
        if !unknown.is_empty() {
            return Err(CrustDbError::Validation(format!(
                "Unknown field(s): {}",
                unknown.join(", ")
            )));
        }

        let mut merged = existing.clone();
        for (field_name, value) in values {
            merged.insert(field_name, value);
        }

        self.normalize_row(schema, &merged)
    }

    fn normalize_row(&self, schema: &ModelSchema, values: &Record) -> Result<Record> {
        let mut row = BTreeMap::new();
        for (field_name, field) in &schema.fields {
            let value = match values.get(field_name) {
                Some(value) => value.clone(),
                None if field.default.is_some() => field.default.clone().unwrap(),
                None if field.required => {
                    return Err(CrustDbError::Validation(format!(
                        "Missing required field: {field_name}"
                    )));
                }
                None => Value::Null,
            };

            self.validate_value(field_name, field, &value)?;
            row.insert(field_name.clone(), value);
        }

        Ok(row)
    }

    fn validate_value(
        &self,
        field_name: &str,
        field: &crate::schema::FieldSchema,
        value: &Value,
    ) -> Result<()> {
        if matches!(value, Value::Null) {
            if field.required {
                return Err(CrustDbError::Validation(format!(
                    "Missing required field: {field_name}"
                )));
            }
            return Ok(());
        }

        match (&field.data_type, value) {
            (DataType::Int, Value::Int(value)) => {
                if let Some((min_value, max_value)) = field.range {
                    if *value < min_value || *value > max_value {
                        return Err(CrustDbError::Validation(format!(
                            "{field_name} must be between {min_value} and {max_value}"
                        )));
                    }
                }
            }
            (DataType::String, Value::String(_)) => {}
            (DataType::Int, other) => {
                return Err(CrustDbError::Validation(format!(
                    "{field_name} must be Int, got {}",
                    other.type_name()
                )));
            }
            (DataType::String, other) => {
                return Err(CrustDbError::Validation(format!(
                    "{field_name} must be String, got {}",
                    other.type_name()
                )));
            }
        }

        Ok(())
    }

    fn validate_unique(
        &self,
        model_name: &str,
        new_row: &Record,
        ignore_row_id: Option<RowId>,
    ) -> Result<()> {
        let schema = self.schema(model_name)?;
        if let Some(indexes) = self.indexes.get(model_name) {
            return indexes.check_unique(schema, new_row, ignore_row_id);
        }

        let Some(rows) = self.rows.get(model_name) else {
            return Ok(());
        };

        for existing in rows.iter().filter(|row| !row.deleted) {
            if Some(existing.row_id) == ignore_row_id {
                continue;
            }

            for (field_name, field) in &schema.fields {
                if field.is_unique() && existing.record.get(field_name) == new_row.get(field_name) {
                    return Err(CrustDbError::UniqueConstraint(format!(
                        "Duplicate value for unique field: {field_name}"
                    )));
                }
            }
        }

        Ok(())
    }

    fn validate_filters(
        &self,
        model_name: &str,
        filters: &Record,
        operation: &str,
    ) -> Result<&ModelSchema> {
        let schema = self.schema(model_name)?;
        if filters.is_empty() {
            return Err(CrustDbError::Validation(format!(
                "{operation} requires at least one field filter"
            )));
        }

        let unknown = unknown_fields(filters.keys(), schema);
        if !unknown.is_empty() {
            return Err(CrustDbError::Validation(format!(
                "Unknown field(s): {}",
                unknown.join(", ")
            )));
        }

        Ok(schema)
    }

    fn rebuild_index(&mut self, model_name: &str) -> Result<()> {
        let schema = self.schema(model_name)?.clone();
        let rows: &[StoredRow] = self.rows.get(model_name).map(Vec::as_slice).unwrap_or(&[]);
        let indexes = disk_index::load_or_rebuild_model(&self.path, model_name, &schema, rows)?;
        self.indexes.insert(model_name.to_string(), indexes);
        Ok(())
    }

    fn live_row_by_id(&self, model_name: &str, row_id: RowId) -> Option<&StoredRow> {
        self.rows
            .get(model_name)?
            .iter()
            .find(|row| row.row_id == row_id && !row.deleted)
    }

    fn live_row_by_id_mut(&mut self, model_name: &str, row_id: RowId) -> Option<&mut StoredRow> {
        self.rows
            .get_mut(model_name)?
            .iter_mut()
            .find(|row| row.row_id == row_id && !row.deleted)
    }

    fn next_row_id(&mut self, model_name: &str) -> RowId {
        self.ensure_next_row_id(model_name);
        self.next_row_ids
            .get(model_name)
            .copied()
            .expect("next row id exists after ensure")
    }

    fn ensure_next_row_id(&mut self, model_name: &str) {
        if self.next_row_ids.contains_key(model_name) {
            return;
        }

        self.next_row_ids.insert(
            model_name.to_string(),
            next_row_id_for(self.rows.get(model_name)),
        );
    }

    fn schema(&self, model_name: &str) -> Result<&ModelSchema> {
        self.schemas
            .get(model_name)
            .ok_or_else(|| CrustDbError::UnknownModel(model_name.to_string()))
    }
}

fn unknown_fields<'a>(
    names: impl Iterator<Item = &'a String>,
    schema: &ModelSchema,
) -> Vec<String> {
    names
        .filter(|field_name| !schema.fields.contains_key(*field_name))
        .cloned()
        .collect()
}

fn next_row_id_for(rows: Option<&Vec<StoredRow>>) -> RowId {
    rows.into_iter()
        .flat_map(|rows| rows.iter())
        .map(|row| row.row_id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DataType, FieldSchema, ModelSchema};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crustdb-{label}-{}-{unique}", std::process::id()))
    }

    fn user_schema() -> ModelSchema {
        ModelSchema {
            name: "User".to_string(),
            fields: BTreeMap::from([
                (
                    "id".to_string(),
                    FieldSchema {
                        data_type: DataType::Int,
                        id: true,
                        unique: false,
                        required: true,
                        range: None,
                        default: None,
                    },
                ),
                (
                    "username".to_string(),
                    FieldSchema {
                        data_type: DataType::String,
                        id: false,
                        unique: true,
                        required: true,
                        range: None,
                        default: None,
                    },
                ),
                (
                    "age".to_string(),
                    FieldSchema {
                        data_type: DataType::Int,
                        id: false,
                        unique: false,
                        required: true,
                        range: Some((0, 120)),
                        default: None,
                    },
                ),
            ]),
        }
    }

    fn user(id: i64, username: &str, age: i64) -> Record {
        BTreeMap::from([
            ("id".to_string(), Value::Int(id)),
            ("username".to_string(), Value::String(username.to_string())),
            ("age".to_string(), Value::Int(age)),
        ])
    }

    #[test]
    fn validates_insert_and_find() {
        let temp = temp_path("engine-basic");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();

        engine.insert("User", user(1, "alice", 25)).unwrap();

        let row = engine
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap()
            .unwrap();

        assert_eq!(
            row.get("username"),
            Some(&Value::String("alice".to_string()))
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rejects_range_violation() {
        let temp = temp_path("engine-range");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();

        let err = engine.insert("User", user(1, "alice", 121)).unwrap_err();

        assert_eq!(err.to_string(), "age must be between 0 and 120");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn reloads_existing_rows_after_reopen() {
        let temp = temp_path("engine-reopen");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        let mut reopened = Engine::open(&temp).unwrap();
        reopened.register_schema(user_schema()).unwrap();
        let row = reopened
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap()
            .unwrap();

        assert_eq!(row.get("age"), Some(&Value::Int(25)));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rejects_incompatible_schema_on_register() {
        let temp = temp_path("engine-schema");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();

        let mut incompatible = user_schema();
        incompatible.fields.get_mut("age").unwrap().data_type = DataType::String;

        let err = engine.register_schema(incompatible).unwrap_err();

        assert!(err.to_string().contains("incompatible"));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn builds_indexes_for_registered_unique_fields() {
        let temp = temp_path("engine-index-build");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let indexes = engine.indexes.get("User").unwrap();

        assert!(indexes.indexes_field("id"));
        assert!(indexes.indexes_field("username"));
        assert!(!indexes.indexes_field("age"));
        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(1));
        assert_eq!(
            indexes.lookup("username", &Value::String("alice".to_string())),
            Some(1)
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rejects_duplicate_primary_key_through_index() {
        let temp = temp_path("engine-index-duplicate");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let err = engine.insert("User", user(1, "bob", 30)).unwrap_err();

        assert_eq!(err.to_string(), "Duplicate value for unique field: id");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn non_indexed_find_still_scans_rows() {
        let temp = temp_path("engine-index-scan");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let row = engine
            .find(
                "User",
                &BTreeMap::from([("age".to_string(), Value::Int(25))]),
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            row.get("username"),
            Some(&Value::String("alice".to_string()))
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn indexes_rebuild_after_reopen() {
        let temp = temp_path("engine-index-reopen");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        let reopened = Engine::open(&temp).unwrap();
        let indexes = reopened.indexes.get("User").unwrap();

        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(1));
        assert_eq!(
            indexes.lookup("username", &Value::String("alice".to_string())),
            Some(1)
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn disk_index_files_are_written_after_insert() {
        let temp = temp_path("engine-disk-index-files");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        assert!(temp.join("indexes").join("manifest.crustix").exists());
        assert!(temp.join("indexes").join("User").join("id.idx").exists());
        assert!(temp
            .join("indexes")
            .join("User")
            .join("username.idx")
            .exists());
        assert!(!temp.join("indexes").join("User").join("age.idx").exists());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn duplicate_persisted_index_value_is_rejected_on_open() {
        let temp = temp_path("engine-index-corrupt-duplicate");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        crate::storage::table_file::append_insert(&temp, "User", 2, &user(1, "bob", 30)).unwrap();
        crate::index::disk::mark_dirty(&temp).unwrap();

        let err = Engine::open(&temp).unwrap_err();

        assert_eq!(err.to_string(), "Duplicate value for unique field: id");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn missing_index_file_rebuilds_from_table_log_on_open() {
        let temp = temp_path("engine-index-missing-rebuild");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        fs::remove_file(temp.join("indexes").join("User").join("id.idx")).unwrap();

        let reopened = Engine::open(&temp).unwrap();

        assert_eq!(
            reopened
                .indexes
                .get("User")
                .unwrap()
                .lookup("id", &Value::Int(1)),
            Some(1)
        );
        assert!(temp.join("indexes").join("User").join("id.idx").exists());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn dirty_index_manifest_rebuilds_from_table_log_on_open() {
        let temp = temp_path("engine-index-dirty-rebuild");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        crate::index::disk::mark_dirty(&temp).unwrap();
        crate::storage::table_file::append_update(&temp, "User", 1, &user(1, "alice2", 26))
            .unwrap();

        let reopened = Engine::open(&temp).unwrap();

        assert_eq!(
            reopened
                .indexes
                .get("User")
                .unwrap()
                .lookup("username", &Value::String("alice".to_string())),
            None
        );
        assert_eq!(
            reopened
                .indexes
                .get("User")
                .unwrap()
                .lookup("username", &Value::String("alice2".to_string())),
            Some(1)
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn row_ids_continue_after_reopen() {
        let temp = temp_path("engine-row-id-reopen");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
        }

        let mut reopened = Engine::open(&temp).unwrap();
        reopened.register_schema(user_schema()).unwrap();
        reopened.insert("User", user(2, "bob", 30)).unwrap();

        let indexes = reopened.indexes.get("User").unwrap();
        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(1));
        assert_eq!(indexes.lookup("id", &Value::Int(2)), Some(2));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn delete_removes_row_from_find_and_indexes() {
        let temp = temp_path("engine-delete");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let deleted = engine
            .delete("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap();

        assert!(deleted);
        assert!(engine
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap()
            .is_none());
        assert_eq!(
            engine
                .indexes
                .get("User")
                .unwrap()
                .lookup("id", &Value::Int(1)),
            None
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn deleted_unique_values_can_be_reused() {
        let temp = temp_path("engine-delete-reuse");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();
        engine
            .delete("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap();

        engine.insert("User", user(2, "alice", 30)).unwrap();

        let row = engine
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(2))]))
            .unwrap()
            .unwrap();
        assert_eq!(
            row.get("username"),
            Some(&Value::String("alice".to_string()))
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn delete_persists_after_reopen() {
        let temp = temp_path("engine-delete-reopen");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
            engine
                .delete("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
                .unwrap();
        }

        let reopened = Engine::open(&temp).unwrap();
        assert!(reopened
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn update_changes_non_indexed_fields() {
        let temp = temp_path("engine-update-age");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let row = engine
            .update(
                "User",
                &BTreeMap::from([("id".to_string(), Value::Int(1))]),
                BTreeMap::from([("age".to_string(), Value::Int(26))]),
            )
            .unwrap()
            .unwrap();

        assert_eq!(row.get("age"), Some(&Value::Int(26)));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn update_changes_indexed_fields() {
        let temp = temp_path("engine-update-indexed");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        engine
            .update(
                "User",
                &BTreeMap::from([("id".to_string(), Value::Int(1))]),
                BTreeMap::from([("username".to_string(), Value::String("alice2".to_string()))]),
            )
            .unwrap();

        assert_eq!(
            engine
                .indexes
                .get("User")
                .unwrap()
                .lookup("username", &Value::String("alice".to_string())),
            None
        );
        assert_eq!(
            engine
                .indexes
                .get("User")
                .unwrap()
                .lookup("username", &Value::String("alice2".to_string())),
            Some(1)
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn update_rejects_duplicate_unique_values() {
        let temp = temp_path("engine-update-duplicate");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();
        engine.insert("User", user(2, "bob", 30)).unwrap();

        let err = engine
            .update(
                "User",
                &BTreeMap::from([("id".to_string(), Value::Int(2))]),
                BTreeMap::from([("username".to_string(), Value::String("alice".to_string()))]),
            )
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Duplicate value for unique field: username"
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn update_allows_unchanged_unique_values_on_same_row() {
        let temp = temp_path("engine-update-same-unique");
        let mut engine = Engine::open(&temp).unwrap();
        engine.register_schema(user_schema()).unwrap();
        engine.insert("User", user(1, "alice", 25)).unwrap();

        let row = engine
            .update(
                "User",
                &BTreeMap::from([("id".to_string(), Value::Int(1))]),
                BTreeMap::from([("age".to_string(), Value::Int(26))]),
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            row.get("username"),
            Some(&Value::String("alice".to_string()))
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn update_persists_after_reopen() {
        let temp = temp_path("engine-update-reopen");
        {
            let mut engine = Engine::open(&temp).unwrap();
            engine.register_schema(user_schema()).unwrap();
            engine.insert("User", user(1, "alice", 25)).unwrap();
            engine
                .update(
                    "User",
                    &BTreeMap::from([("id".to_string(), Value::Int(1))]),
                    BTreeMap::from([("age".to_string(), Value::Int(26))]),
                )
                .unwrap();
        }

        let reopened = Engine::open(&temp).unwrap();
        let row = reopened
            .find("User", &BTreeMap::from([("id".to_string(), Value::Int(1))]))
            .unwrap()
            .unwrap();

        assert_eq!(row.get("age"), Some(&Value::Int(26)));
        let _ = fs::remove_dir_all(&temp);
    }
}
