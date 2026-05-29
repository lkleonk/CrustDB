use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    Int,
    String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSchema {
    #[serde(rename = "type")]
    pub data_type: DataType,
    pub id: bool,
    pub unique: bool,
    pub required: bool,
    pub range: Option<(i64, i64)>,
    pub default: Option<Value>,
}

impl FieldSchema {
    pub fn is_unique(&self) -> bool {
        self.id || self.unique
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSchema {
    pub name: String,
    pub fields: BTreeMap<String, FieldSchema>,
}
