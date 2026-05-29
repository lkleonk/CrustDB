use crate::error::{CrustDbError, Result};
use crate::value::Value;

const KEY_NULL: u8 = 0;
const KEY_INT: u8 = 1;
const KEY_STRING: u8 = 2;
const KEY_BOOL: u8 = 3;

pub(crate) fn encode_key(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => vec![KEY_NULL],
        Value::Int(value) => {
            let sortable = (*value as u64) ^ 0x8000_0000_0000_0000;
            let mut out = vec![KEY_INT];
            out.extend_from_slice(&sortable.to_be_bytes());
            out
        }
        Value::String(value) => {
            let mut out = vec![KEY_STRING];
            out.extend_from_slice(value.as_bytes());
            out
        }
        Value::Bool(value) => vec![KEY_BOOL, u8::from(*value)],
    }
}

pub(crate) fn decode_key(bytes: &[u8]) -> Result<Value> {
    let Some((tag, payload)) = bytes.split_first() else {
        return Err(CrustDbError::StorageFormat("empty index key".to_string()));
    };

    match *tag {
        KEY_NULL if payload.is_empty() => Ok(Value::Null),
        KEY_INT if payload.len() == 8 => {
            let sortable = u64::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
                payload[7],
            ]);
            Ok(Value::Int((sortable ^ 0x8000_0000_0000_0000) as i64))
        }
        KEY_STRING => String::from_utf8(payload.to_vec())
            .map(Value::String)
            .map_err(|_| CrustDbError::StorageFormat("index string key is not UTF-8".to_string())),
        KEY_BOOL if payload.len() == 1 => match payload[0] {
            0 => Ok(Value::Bool(false)),
            1 => Ok(Value::Bool(true)),
            other => Err(CrustDbError::StorageFormat(format!(
                "invalid index bool key: {other}"
            ))),
        },
        other => Err(CrustDbError::StorageFormat(format!(
            "invalid index key tag or payload: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_keys_sort_in_numeric_order() {
        let values = [Value::Int(-2), Value::Int(-1), Value::Int(0), Value::Int(1)];
        let mut encoded: Vec<_> = values.iter().map(encode_key).collect();
        encoded.sort();

        let decoded: Vec<_> = encoded.iter().map(|key| decode_key(key).unwrap()).collect();

        assert_eq!(decoded, values);
    }

    #[test]
    fn string_keys_sort_lexically() {
        let mut encoded = [
            Value::String("bob".to_string()),
            Value::String("alice".to_string()),
            Value::String("alice2".to_string()),
        ]
        .iter()
        .map(encode_key)
        .collect::<Vec<_>>();
        encoded.sort();

        let decoded = encoded
            .iter()
            .map(|key| decode_key(key).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            decoded,
            vec![
                Value::String("alice".to_string()),
                Value::String("alice2".to_string()),
                Value::String("bob".to_string())
            ]
        );
    }

    #[test]
    fn different_value_types_do_not_collide() {
        assert_ne!(
            encode_key(&Value::Int(1)),
            encode_key(&Value::String("1".to_string()))
        );
        assert_ne!(encode_key(&Value::Null), encode_key(&Value::Bool(false)));
    }
}
