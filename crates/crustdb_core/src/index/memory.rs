use crate::engine::{Record, RowId, StoredRow};
use crate::error::{CrustDbError, Result};
use crate::schema::ModelSchema;
use crate::value::Value;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub(crate) struct TableIndexes {
    by_field: HashMap<String, HashMap<Value, RowId>>,
}

impl TableIndexes {
    pub(crate) fn build(schema: &ModelSchema, rows: &[StoredRow]) -> Result<Self> {
        let mut indexes = Self::for_schema(schema);
        for row in rows.iter().filter(|row| !row.deleted) {
            indexes.check_unique(schema, &row.record, None)?;
            indexes.insert(schema, row.row_id, &row.record);
        }
        Ok(indexes)
    }

    pub(crate) fn check_unique(
        &self,
        schema: &ModelSchema,
        row: &Record,
        ignore_row_id: Option<RowId>,
    ) -> Result<()> {
        for (field_name, field) in &schema.fields {
            if !field.is_unique() {
                continue;
            }

            let Some(value) = row.get(field_name) else {
                continue;
            };

            let Some(existing_row_id) = self
                .by_field
                .get(field_name)
                .and_then(|index| index.get(value))
                .copied()
            else {
                continue;
            };

            if Some(existing_row_id) != ignore_row_id {
                return Err(CrustDbError::UniqueConstraint(format!(
                    "Duplicate value for unique field: {field_name}"
                )));
            }
        }

        Ok(())
    }

    pub(crate) fn insert(&mut self, schema: &ModelSchema, row_id: RowId, row: &Record) {
        for (field_name, field) in &schema.fields {
            if !field.is_unique() {
                continue;
            }

            let Some(value) = row.get(field_name) else {
                continue;
            };

            if let Some(index) = self.by_field.get_mut(field_name) {
                index.insert(value.clone(), row_id);
            }
        }
    }

    pub(crate) fn remove(&mut self, schema: &ModelSchema, row_id: RowId, row: &Record) {
        for (field_name, field) in &schema.fields {
            if !field.is_unique() {
                continue;
            }

            let Some(value) = row.get(field_name) else {
                continue;
            };

            if let Some(index) = self.by_field.get_mut(field_name) {
                if index.get(value) == Some(&row_id) {
                    index.remove(value);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn lookup(&self, field_name: &str, value: &Value) -> Option<RowId> {
        self.by_field
            .get(field_name)
            .and_then(|index| index.get(value))
            .copied()
    }

    pub(crate) fn indexes_field(&self, field_name: &str) -> bool {
        self.by_field.contains_key(field_name)
    }

    pub(crate) fn indexed_field_names(schema: &ModelSchema) -> Vec<&str> {
        schema
            .fields
            .iter()
            .filter(|(_, field)| field.is_unique())
            .map(|(field_name, _)| field_name.as_str())
            .collect()
    }

    pub(crate) fn entries_for_field(&self, field_name: &str) -> Vec<(Value, RowId)> {
        self.by_field
            .get(field_name)
            .map(|entries| {
                entries
                    .iter()
                    .map(|(value, row_id)| (value.clone(), *row_id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn from_entries(
        schema: &ModelSchema,
        entries_by_field: HashMap<String, Vec<(Value, RowId)>>,
    ) -> Result<Self> {
        let mut indexes = Self::for_schema(schema);

        for (field_name, entries) in entries_by_field {
            let Some(field_index) = indexes.by_field.get_mut(&field_name) else {
                return Err(CrustDbError::StorageFormat(format!(
                    "unexpected persisted index field: {field_name}"
                )));
            };

            for (value, row_id) in entries {
                if field_index.insert(value, row_id).is_some() {
                    return Err(CrustDbError::StorageFormat(format!(
                        "duplicate key in persisted index: {field_name}"
                    )));
                }
            }
        }

        Ok(indexes)
    }

    fn for_schema(schema: &ModelSchema) -> Self {
        let by_field = schema
            .fields
            .iter()
            .filter(|(_, field)| field.is_unique())
            .map(|(field_name, _)| (field_name.clone(), HashMap::new()))
            .collect();

        Self { by_field }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DataType, FieldSchema};
    use std::collections::BTreeMap;

    fn schema() -> ModelSchema {
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
                        range: None,
                        default: None,
                    },
                ),
            ]),
        }
    }

    fn record(id: i64, username: &str, age: i64) -> Record {
        BTreeMap::from([
            ("id".to_string(), Value::Int(id)),
            ("username".to_string(), Value::String(username.to_string())),
            ("age".to_string(), Value::Int(age)),
        ])
    }

    fn stored(row_id: RowId, record: Record) -> StoredRow {
        StoredRow {
            row_id,
            record,
            deleted: false,
        }
    }

    #[test]
    fn builds_indexes_for_id_and_unique_fields_only() {
        let schema = schema();
        let indexes = TableIndexes::build(&schema, &[stored(7, record(1, "alice", 25))]).unwrap();

        assert!(indexes.indexes_field("id"));
        assert!(indexes.indexes_field("username"));
        assert!(!indexes.indexes_field("age"));
        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(7));
        assert_eq!(
            indexes.lookup("username", &Value::String("alice".to_string())),
            Some(7)
        );
    }

    #[test]
    fn rejects_duplicate_indexed_values_while_building() {
        let schema = schema();
        let err = TableIndexes::build(
            &schema,
            &[
                stored(7, record(1, "alice", 25)),
                stored(8, record(1, "bob", 30)),
            ],
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "Duplicate value for unique field: id");
    }

    #[test]
    fn ignores_same_row_id_when_checking_unique_values() {
        let schema = schema();
        let indexes = TableIndexes::build(&schema, &[stored(7, record(1, "alice", 25))]).unwrap();

        indexes
            .check_unique(&schema, &record(1, "alice", 26), Some(7))
            .unwrap();
    }

    #[test]
    fn remove_deletes_indexed_values_for_that_row_only() {
        let schema = schema();
        let mut indexes =
            TableIndexes::build(&schema, &[stored(7, record(1, "alice", 25))]).unwrap();

        indexes.remove(&schema, 8, &record(1, "alice", 25));
        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(7));

        indexes.remove(&schema, 7, &record(1, "alice", 25));
        assert_eq!(indexes.lookup("id", &Value::Int(1)), None);
    }

    #[test]
    fn builds_from_persisted_entries() {
        let schema = schema();
        let indexes = TableIndexes::from_entries(
            &schema,
            HashMap::from([
                ("id".to_string(), vec![(Value::Int(1), 7)]),
                (
                    "username".to_string(),
                    vec![(Value::String("alice".to_string()), 7)],
                ),
            ]),
        )
        .unwrap();

        assert_eq!(indexes.lookup("id", &Value::Int(1)), Some(7));
        assert_eq!(
            indexes.lookup("username", &Value::String("alice".to_string())),
            Some(7)
        );
    }
}
