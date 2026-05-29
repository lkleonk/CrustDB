use crate::error::{CrustDbError, Result};
use crate::schema::{DataType, FieldSchema, ModelSchema};
use crate::storage::record::{write_i64, write_string, write_u32, write_value, Cursor};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

const SCHEMA_MAGIC: &[u8; 8] = b"CRUSTSC1";
const TYPE_INT: u8 = 1;
const TYPE_STRING: u8 = 2;

pub fn load_schemas(root: &Path) -> Result<HashMap<String, ModelSchema>> {
    let path = root.join("schema.crust");
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let bytes = fs::read(path)?;
    let mut cursor = Cursor::new(&bytes);
    for expected in SCHEMA_MAGIC {
        let found = cursor.read_u8()?;
        if found != *expected {
            return Err(CrustDbError::StorageFormat(
                "invalid schema file magic".to_string(),
            ));
        }
    }

    let model_count = cursor.read_u32()? as usize;
    let mut schemas = HashMap::new();

    for _ in 0..model_count {
        let model_name = cursor.read_string()?;
        let field_count = cursor.read_u32()? as usize;
        let mut fields = BTreeMap::new();

        for _ in 0..field_count {
            let field_name = cursor.read_string()?;
            let data_type = match cursor.read_u8()? {
                TYPE_INT => DataType::Int,
                TYPE_STRING => DataType::String,
                other => {
                    return Err(CrustDbError::StorageFormat(format!(
                        "unknown schema field type: {other}"
                    )));
                }
            };

            let id = read_bool(&mut cursor, "id")?;
            let unique = read_bool(&mut cursor, "unique")?;
            let required = read_bool(&mut cursor, "required")?;

            let has_range = read_bool(&mut cursor, "range marker")?;
            let range = if has_range {
                Some((cursor.read_i64()?, cursor.read_i64()?))
            } else {
                None
            };

            let has_default = read_bool(&mut cursor, "default marker")?;
            let default = if has_default {
                Some(cursor.read_value()?)
            } else {
                None
            };

            fields.insert(
                field_name,
                FieldSchema {
                    data_type,
                    id,
                    unique,
                    required,
                    range,
                    default,
                },
            );
        }

        schemas.insert(
            model_name.clone(),
            ModelSchema {
                name: model_name,
                fields,
            },
        );
    }

    if !cursor.is_finished() {
        return Err(CrustDbError::StorageFormat(
            "schema file has trailing bytes".to_string(),
        ));
    }

    Ok(schemas)
}

pub fn save_schemas(root: &Path, schemas: &HashMap<String, ModelSchema>) -> Result<()> {
    let mut out = Vec::new();
    out.extend_from_slice(SCHEMA_MAGIC);

    let sorted: BTreeMap<_, _> = schemas
        .iter()
        .map(|(name, schema)| (name.clone(), schema.clone()))
        .collect();
    write_u32(
        &mut out,
        crate::storage::record::usize_to_u32(sorted.len(), "model count")?,
    );

    for (model_name, schema) in sorted {
        write_string(&mut out, &model_name)?;
        write_u32(
            &mut out,
            crate::storage::record::usize_to_u32(schema.fields.len(), "field count")?,
        );

        for (field_name, field) in schema.fields {
            write_string(&mut out, &field_name)?;
            out.push(match field.data_type {
                DataType::Int => TYPE_INT,
                DataType::String => TYPE_STRING,
            });
            out.push(u8::from(field.id));
            out.push(u8::from(field.unique));
            out.push(u8::from(field.required));

            match field.range {
                Some((min_value, max_value)) => {
                    out.push(1);
                    write_i64(&mut out, min_value);
                    write_i64(&mut out, max_value);
                }
                None => out.push(0),
            }

            match field.default {
                Some(default) => {
                    out.push(1);
                    write_value(&mut out, &default)?;
                }
                None => out.push(0),
            }
        }
    }

    fs::write(root.join("schema.crust"), out)?;
    Ok(())
}

fn read_bool(cursor: &mut Cursor<'_>, name: &str) -> Result<bool> {
    match cursor.read_u8()? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(CrustDbError::StorageFormat(format!(
            "invalid {name} bool: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("crustdb-{label}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn schema_file_save_load_roundtrip() {
        let root = temp_path("schema-roundtrip");
        fs::create_dir_all(&root).unwrap();
        let schema = ModelSchema {
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
                    "name".to_string(),
                    FieldSchema {
                        data_type: DataType::String,
                        id: false,
                        unique: true,
                        required: true,
                        range: None,
                        default: Some(Value::String("untitled".to_string())),
                    },
                ),
            ]),
        };
        let schemas = HashMap::from([("User".to_string(), schema)]);

        save_schemas(&root, &schemas).unwrap();
        let loaded = load_schemas(&root).unwrap();

        assert_eq!(loaded, schemas);
        let _ = fs::remove_dir_all(root);
    }
}
