//! Decoders for the Hivemind/Petals DHT record wire format.
//!
//! Records reach us as msgpack extension types. `Ext(64)` is a serialised
//! value; `Ext(80)` is a dictionary of subkeyed values, which is how many
//! servers accumulate under one block key instead of overwriting each other.
//! The layouts are a published format shared with Python Hivemind — see
//! `kwaai-cli/src/announce.rs` for the writer's side.

use rmpv::Value;
use sha1::{Digest, Sha1};

use crate::snapshot::ServerInfo;

/// Hivemind's `DHTID.generate()`: SHA1 over the msgpack encoding of the key.
pub fn dht_key(raw_key: &str) -> Vec<u8> {
    let packed = rmp_serde::to_vec(raw_key).expect("msgpack of a &str cannot fail");
    Sha1::new().chain_update(&packed).finalize().to_vec()
}

/// Unwrap one msgpack extension type, returning its payload decoded.
fn unwrap_ext(bytes: &[u8], tag: i8) -> Option<Value> {
    let outer = rmpv::decode::read_value(&mut &bytes[..]).ok()?;
    let payload = match &outer {
        Value::Ext(t, b) if *t == tag => b.as_slice(),
        _ => return None,
    };
    rmpv::decode::read_value(&mut &payload[..]).ok()
}

/// A subkey arrives either as a plain string or as msgpack-in-binary.
fn subkey_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => s.as_str().map(str::to_string),
        Value::Binary(b) => match rmpv::decode::read_value(&mut b.as_slice()) {
            Ok(Value::String(s)) => s.as_str().map(str::to_string),
            _ => None,
        },
        _ => None,
    }
}

/// Entries of an `Ext(80)` dictionary value as `(subkey, raw value bytes)`.
pub fn dictionary_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Some(inner) = unwrap_ext(bytes, 80) else {
        return Vec::new();
    };
    let Some(arr) = inner.as_array() else {
        return Vec::new();
    };
    // [latest_expiration, ?, [[subkey, value, expiration], ...]]
    let Some(entries) = arr.get(2).and_then(Value::as_array) else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let e = entry.as_array()?;
            let subkey = subkey_to_string(e.first()?)?;
            // Values are always stored as opaque bytes; anything else is
            // not a record we can decode.
            let Value::Binary(value) = e.get(1)? else {
                return None;
            };
            let value = value.clone();
            (!subkey.is_empty()).then_some((subkey, value))
        })
        .collect()
}

/// Petals `ServerState`: 0 offline, 1 joining, 2 online. KwaaiNet also writes
/// -1 as an explicit "remove me now" tombstone.
fn state_name(raw: i64) -> &'static str {
    match raw {
        -1 | 0 => "offline",
        1 => "joining",
        2 => "online",
        _ => "unknown",
    }
}

/// Decode an `Ext(64)` server record: `[state, throughput, {fields}]`.
pub fn decode_server_info(bytes: &[u8]) -> Option<ServerInfo> {
    let inner = unwrap_ext(bytes, 64)?;
    let arr = inner.as_array()?;
    if arr.len() < 3 {
        return None;
    }

    let mut info = ServerInfo {
        state: state_name(arr[0].as_i64().unwrap_or(0)).to_string(),
        throughput: arr[1].as_f64().unwrap_or(0.0),
        ..Default::default()
    };

    let s = |v: &Value| v.as_str().map(str::to_string);
    for (k, v) in arr[2].as_map()? {
        match k.as_str().unwrap_or("") {
            "start_block" => info.start_block = v.as_i64().unwrap_or(0),
            "end_block" => info.end_block = v.as_i64().unwrap_or(0),
            "public_name" => info.public_name = s(v),
            "version" => info.version = s(v),
            "network_rps" => info.network_rps = v.as_f64(),
            "forward_rps" => info.forward_rps = v.as_f64(),
            "inference_rps" => info.inference_rps = v.as_f64(),
            "torch_dtype" => info.torch_dtype = s(v),
            "quant_type" => info.quant_type = s(v),
            "using_relay" => info.using_relay = v.as_bool(),
            "shard_loading" => info.shard_loading = v.as_bool(),
            "cache_tokens_left" => info.cache_tokens_left = v.as_i64(),
            "peer_id" => info.peer_id = s(v),
            "vpk" => info.vpk = serde_json::to_value(v).ok(),
            "trust_attestations" => {
                info.trust_attestations = v.as_array().map(Vec::len).unwrap_or(0)
            }
            _ => {}
        }
    }
    Some(info)
}

/// One entry of the `_petals.models` registry.
pub struct ModelRegistration {
    pub dht_prefix: String,
    pub repository: String,
    pub num_blocks: i64,
}

/// Decode the `_petals.models` dictionary: subkey is the prefix, value is a
/// plain msgpack map of `{repository, num_blocks}` — not an `Ext(64)`.
pub fn decode_model_registry(bytes: &[u8]) -> Vec<ModelRegistration> {
    dictionary_entries(bytes)
        .into_iter()
        .filter_map(|(dht_prefix, value)| {
            let v = rmpv::decode::read_value(&mut &value[..]).ok()?;
            let map = v.as_map()?;
            let find = |name: &str| map.iter().find(|(k, _)| k.as_str() == Some(name));
            Some(ModelRegistration {
                dht_prefix,
                repository: find("repository")
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                num_blocks: find("num_blocks")
                    .and_then(|(_, v)| v.as_i64())
                    .unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a payload the way the DHT does, so tests exercise the real path.
    fn ext(tag: i8, payload: Value) -> Vec<u8> {
        let mut inner = Vec::new();
        rmpv::encode::write_value(&mut inner, &payload).unwrap();
        let mut out = Vec::new();
        rmpv::encode::write_value(&mut out, &Value::Ext(tag, inner)).unwrap();
        out
    }

    #[test]
    fn dht_key_matches_hivemind() {
        // SHA1(msgpack("_petals.models")) — the value Python Hivemind derives.
        assert_eq!(hex::encode(dht_key("_petals.models")).len(), 40);
        assert_ne!(dht_key("a"), dht_key("b"));
    }

    #[test]
    fn decodes_a_server_record() {
        let record = ext(
            64,
            Value::Array(vec![
                Value::from(2),
                Value::from(31.5),
                Value::Map(vec![
                    (Value::from("start_block"), Value::from(0)),
                    (Value::from("end_block"), Value::from(15)),
                    (Value::from("public_name"), Value::from("alice")),
                    (Value::from("using_relay"), Value::from(true)),
                ]),
            ]),
        );

        let info = decode_server_info(&record).expect("decodes");
        assert_eq!(info.state, "online");
        assert_eq!(info.throughput, 31.5);
        assert_eq!(info.end_block, 15);
        assert_eq!(info.public_name.as_deref(), Some("alice"));
        assert_eq!(info.using_relay, Some(true));
    }

    #[test]
    fn shard_loading_is_read_when_present_and_absent_is_not_false_positive() {
        let with = ext(
            64,
            Value::Array(vec![
                Value::from(1),
                Value::from(0.0),
                Value::Map(vec![(Value::from("shard_loading"), Value::from(true))]),
            ]),
        );
        assert_eq!(decode_server_info(&with).unwrap().shard_loading, Some(true));

        let without = ext(
            64,
            Value::Array(vec![
                Value::from(1),
                Value::from(0.0),
                Value::Map(vec![(Value::from("start_block"), Value::from(0))]),
            ]),
        );
        assert_eq!(decode_server_info(&without).unwrap().shard_loading, None);
    }

    #[test]
    fn tombstone_state_reads_as_offline() {
        let record = ext(
            64,
            Value::Array(vec![
                Value::from(-1),
                Value::from(0.0),
                Value::Map(vec![(Value::from("start_block"), Value::from(0))]),
            ]),
        );
        assert_eq!(decode_server_info(&record).unwrap().state, "offline");
    }

    #[test]
    fn decodes_model_registry() {
        let entry = {
            let mut v = Vec::new();
            rmpv::encode::write_value(
                &mut v,
                &Value::Map(vec![
                    (
                        Value::from("repository"),
                        Value::from("https://huggingface.co/unsloth/Llama-3.1-8B-Instruct"),
                    ),
                    (Value::from("num_blocks"), Value::from(32)),
                ]),
            )
            .unwrap();
            v
        };
        let dict = ext(
            80,
            Value::Array(vec![
                Value::from(0.0),
                Value::Nil,
                Value::Array(vec![Value::Array(vec![
                    Value::from("Llama-3-1-8B-Instruct"),
                    Value::Binary(entry),
                    Value::from(0.0),
                ])]),
            ]),
        );

        let models = decode_model_registry(&dict);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].dht_prefix, "Llama-3-1-8B-Instruct");
        assert_eq!(models[0].num_blocks, 32);
    }
}
