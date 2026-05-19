use gbf_foundation::Hash256;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let mut canonical_bytes = Vec::new();
    write_canonical_json_value(&value, &mut canonical_bytes)?;
    Ok(canonical_bytes)
}

pub(crate) fn domain_hash<T: Serialize>(
    crate_name: &str,
    type_name: &str,
    version: &str,
    value: &T,
) -> Result<Hash256, serde_json::Error> {
    let mut hasher = Sha256::new();
    hasher.update(format!("gbf:{crate_name}:{type_name}:{version}\0"));
    hasher.update(canonical_json_bytes(value)?);
    Ok(Hash256::from_bytes(hasher.finalize().into()))
}

fn write_canonical_json_value(
    value: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_writer(out, value)?,
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical_json_value(item, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(entries) => {
            out.push(b'{');
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_json_value(&entries[*key], out)?;
            }
            out.push(b'}');
        }
    }

    Ok(())
}
