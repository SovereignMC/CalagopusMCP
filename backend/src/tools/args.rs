use serde_json::{Map, Value};

/// Largest accepted page number. Only there so a nonsense value is rejected with a range message
/// instead of being passed to the panel.
pub const MAX_PAGE: i64 = 2_147_483_647;

pub fn to_map(value: &Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map.clone()),
        Value::Null => Ok(Map::new()),
        _ => Err("`arguments` must be an object".to_string()),
    }
}

pub fn text(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(format!("`{key}` must be a string")),
        None => Err(format!("`{key}` is required")),
    }
}

pub fn optional_text(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

pub fn integer(
    args: &Map<String, Value>,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
) -> Result<i64, String> {
    let value = match args.get(key) {
        None | Some(Value::Null) => default,
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| format!("`{key}` must be a whole number"))?,
        Some(_) => return Err(format!("`{key}` must be a whole number")),
    };

    if value < min || value > max {
        return Err(format!("`{key}` must be between {min} and {max}"));
    }

    Ok(value)
}

pub fn boolean(
    args: &Map<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

pub fn object(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<Map<String, Value>>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map.clone())),
        Some(_) => Err(format!("`{key}` must be an object")),
    }
}

pub fn string_list(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("`{key}` must be a list of strings"))
            })
            .collect(),
        Some(_) => Err(format!("`{key}` must be a list of strings")),
    }
}

/// An identifier that is interpolated into a panel URL path. Rejecting separators and whitespace
/// keeps a caller from appending to or breaking out of the constructed route.
pub fn server(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    let identifier = text(args, key)?;

    if identifier.is_empty() {
        return Err(format!("`{key}` must not be empty"));
    }

    if identifier
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '/' | '?' | '#'))
    {
        return Err(format!(
            "`{key}` must be an identifier, not a path or a query"
        ));
    }

    Ok(identifier)
}

pub fn choice(
    args: &Map<String, Value>,
    key: &str,
    default: Option<&str>,
    allowed: &[&str],
) -> Result<String, String> {
    let value = match args.get(key) {
        None | Some(Value::Null) => match default {
            Some(default) => default.to_string(),
            None => {
                return Err(format!("`{key}` must be {}", listed(allowed)));
            }
        },
        Some(Value::String(value)) => value.clone(),
        Some(_) => return Err(format!("`{key}` must be {}", listed(allowed))),
    };

    if !allowed.contains(&value.as_str()) {
        return Err(format!("`{key}` must be {}", listed(allowed)));
    }

    Ok(value)
}

fn listed(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}
