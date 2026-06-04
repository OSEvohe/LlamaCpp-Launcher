/// Coerce a JSON value to ``i64``. Booleans are **rejected** (legacy parity).
pub(crate) fn coerce_int(val: &serde_json::Value, field: &str) -> Result<i64, String> {
    match val {
        serde_json::Value::Bool(_) => Err(format!("{} must be an integer", field)),
        serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| format!("{} must be an integer", field)),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("{} must be an integer", field));
            }
            trimmed.parse::<i64>().map_err(|_| format!("{} must be an integer", field))
        }
        _ => Err(format!("{} must be an integer", field)),
    }
}

/// Coerce a JSON value to ``f64``. Booleans are **rejected** (legacy parity).
pub(crate) fn coerce_float(val: &serde_json::Value, field: &str) -> Result<f64, String> {
    match val {
        serde_json::Value::Bool(_) => Err(format!("{} must be a number", field)),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                Ok(f)
            } else {
                Err(format!("{} must be a number", field))
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("{} must be a number", field));
            }
            trimmed.parse::<f64>().map_err(|_| format!("{} must be a number", field))
        }
        _ => Err(format!("{} must be a number", field)),
    }
}

/// Coerce a JSON value to ``bool``. Accepts ``"true"``/``"false"`` strings.
pub(crate) fn coerce_bool(val: &serde_json::Value, field: &str) -> Result<bool, String> {
    match val {
        serde_json::Value::Bool(b) => Ok(*b),
        serde_json::Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("{} must be a boolean", field)),
        },
        _ => Err(format!("{} must be a boolean", field)),
    }
}
