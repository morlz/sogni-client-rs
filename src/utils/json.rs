use serde_json::Value;

#[must_use]
pub fn drop_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(key, value)| (!value.is_null()).then(|| (key, drop_nulls(value))))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(drop_nulls).collect()),
        value => value,
    }
}

pub(crate) fn query_pairs(query: &Value) -> Vec<(String, String)> {
    let Value::Object(map) = query else {
        return Vec::new();
    };
    map.iter()
        .filter(|(_, value)| !value.is_null())
        .flat_map(|(key, value)| match value {
            Value::Array(items) => items
                .iter()
                .map(|item| (key.clone(), scalar_string(item)))
                .collect::<Vec<_>>(),
            value => vec![(key.clone(), scalar_string(value))],
        })
        .collect()
}

pub(crate) fn scalar_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        value => serde_json::to_string(value).unwrap_or_default(),
    }
}
