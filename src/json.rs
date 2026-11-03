//! Convert deserialized Unity objects to JSON without losing maps whose keys aren't strings.

use serde_json::{Map, Number, Value};

/// Maps with only string keys become objects, others (like `m_RenderDataMap`'s
/// `pair<GUID, long>` keys) arrays of `[key, value]`. Non-finite floats become strings.
pub fn to_json(value: serde_value::Value) -> Value {
    use serde_value::Value as V;
    match value {
        V::Bool(v) => Value::Bool(v),
        V::U8(v) => v.into(),
        V::U16(v) => v.into(),
        V::U32(v) => v.into(),
        V::U64(v) => v.into(),
        V::I8(v) => v.into(),
        V::I16(v) => v.into(),
        V::I32(v) => v.into(),
        V::I64(v) => v.into(),
        V::F32(v) => float(v.into()),
        V::F64(v) => float(v),
        V::Char(v) => Value::String(v.into()),
        V::String(v) => Value::String(v),
        V::Unit => Value::Null,
        V::Option(v) => v.map_or(Value::Null, |v| to_json(*v)),
        V::Newtype(v) => to_json(*v),
        V::Seq(items) => Value::Array(items.into_iter().map(to_json).collect()),
        V::Bytes(bytes) => Value::Array(bytes.into_iter().map(Value::from).collect()),
        V::Map(map) if map.keys().all(|key| matches!(key, V::String(_))) => Value::Object(
            map.into_iter()
                .map(|(key, value)| match key {
                    V::String(key) => (key, to_json(value)),
                    _ => unreachable!(),
                })
                .collect::<Map<_, _>>(),
        ),
        V::Map(map) => Value::Array(
            map.into_iter()
                .map(|(key, value)| Value::Array(vec![to_json(key), to_json(value)]))
                .collect(),
        ),
    }
}

fn float(v: f64) -> Value {
    Number::from_f64(v).map_or_else(|| Value::String(v.to_string()), Value::Number)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use serde_value::{Map, Value as V};

    use super::to_json;

    #[test]
    fn maps_with_string_keys_become_objects() {
        let map: Map<V, V> = [(V::String("a".into()), V::I32(1))].into_iter().collect();
        assert_eq!(to_json(V::Map(map)), json!({"a": 1}));
    }

    #[test]
    fn maps_with_other_keys_become_pairs() {
        let key: Map<V, V> = [(V::String("first".into()), V::U32(7))]
            .into_iter()
            .collect();
        let map: Map<V, V> = [(V::Map(key), V::I64(21300000))].into_iter().collect();
        assert_eq!(to_json(V::Map(map)), json!([[{"first": 7}, 21300000]]));
    }

    #[test]
    fn non_finite_floats_become_strings() {
        assert_eq!(to_json(V::F32(f32::NAN)), json!("NaN"));
        assert_eq!(to_json(V::F64(f64::INFINITY)), json!("inf"));
    }
}
