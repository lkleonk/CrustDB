use crate::engine::Record;
use crate::error::{CrustDbError, Result};
use crate::value::Value;

const VALUE_NULL: u8 = 0;
const VALUE_INT: u8 = 1;
const VALUE_STRING: u8 = 2;
const VALUE_BOOL: u8 = 3;

pub fn encode_record(record: &Record) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_u32(&mut out, usize_to_u32(record.len(), "record field count")?);

    for (field_name, value) in record {
        write_string(&mut out, field_name)?;
        write_value(&mut out, value)?;
    }

    Ok(out)
}

pub fn decode_record(bytes: &[u8]) -> Result<Record> {
    let mut cursor = Cursor::new(bytes);
    let field_count = cursor.read_u32()? as usize;
    let mut record = Record::new();

    for _ in 0..field_count {
        let field_name = cursor.read_string()?;
        let value = cursor.read_value()?;
        record.insert(field_name, value);
    }

    if !cursor.is_finished() {
        return Err(CrustDbError::StorageFormat(
            "record has trailing bytes".to_string(),
        ));
    }

    Ok(record)
}

pub(crate) fn write_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    write_u16(out, usize_to_u16(bytes.len(), "string length")?);
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn write_value(out: &mut Vec<u8>, value: &Value) -> Result<()> {
    match value {
        Value::Null => out.push(VALUE_NULL),
        Value::Int(value) => {
            out.push(VALUE_INT);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::String(value) => {
            out.push(VALUE_STRING);
            write_string(out, value)?;
        }
        Value::Bool(value) => {
            out.push(VALUE_BOOL);
            out.push(u8::from(*value));
        }
    }

    Ok(())
}

pub(crate) fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn usize_to_u16(value: usize, name: &str) -> Result<u16> {
    u16::try_from(value).map_err(|_| {
        CrustDbError::StorageFormat(format!("{name} is too large for this storage format"))
    })
}

pub(crate) fn usize_to_u32(value: usize, name: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| {
        CrustDbError::StorageFormat(format!("{name} is too large for this storage format"))
    })
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.position == self.bytes.len()
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        let bytes = self.read_exact(1)?;
        Ok(bytes[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn read_i64(&mut self) -> Result<i64> {
        let bytes = self.read_exact(8)?;
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn read_string(&mut self) -> Result<String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| CrustDbError::StorageFormat("string value is not valid UTF-8".to_string()))
    }

    pub(crate) fn read_value(&mut self) -> Result<Value> {
        match self.read_u8()? {
            VALUE_NULL => Ok(Value::Null),
            VALUE_INT => Ok(Value::Int(self.read_i64()?)),
            VALUE_STRING => Ok(Value::String(self.read_string()?)),
            VALUE_BOOL => match self.read_u8()? {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                other => Err(CrustDbError::StorageFormat(format!(
                    "invalid bool payload: {other}"
                ))),
            },
            other => Err(CrustDbError::StorageFormat(format!(
                "unknown value tag: {other}"
            ))),
        }
    }

    pub(crate) fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| CrustDbError::StorageFormat("record cursor overflow".to_string()))?;

        if end > self.bytes.len() {
            return Err(CrustDbError::StorageFormat(
                "unexpected end of record".to_string(),
            ));
        }

        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn record_encode_decode_roundtrip() {
        let record = BTreeMap::from([
            ("age".to_string(), Value::Int(25)),
            ("active".to_string(), Value::Bool(true)),
            ("nickname".to_string(), Value::Null),
            ("username".to_string(), Value::String("alice".to_string())),
        ]);

        let encoded = encode_record(&record).unwrap();
        let decoded = decode_record(&encoded).unwrap();

        assert_eq!(decoded, record);
    }

    #[test]
    fn corrupt_record_returns_storage_error() {
        let err = decode_record(&[1, 0, 0, 0, 5]).unwrap_err();

        assert!(err.to_string().contains("Storage format error"));
    }
}
