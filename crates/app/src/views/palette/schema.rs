//! JSON-schema parameter fields with explicit, lossless scalar conversion.
use serde_json::{Map, Value};
#[derive(Debug, Clone)]
pub(super) struct Field {
    pub name: String,
    pub required: bool,
    pub description: String,
    pub schema: Value,
}
fn resolve<'a>(root: &'a Value, value: &'a Value) -> &'a Value {
    if let Some(reference) = value
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix('#'))
        && let Some(value) = root.pointer(reference)
    {
        return resolve(root, value);
    }
    if let Some(branches) = value
        .get("anyOf")
        .or_else(|| value.get("oneOf"))
        .and_then(Value::as_array)
        && let Some(branch) = branches
            .iter()
            .find(|v| v.get("type").and_then(Value::as_str) != Some("null"))
    {
        return resolve(root, branch);
    }
    value
}
pub(super) fn fields(root: &Value) -> Vec<Field> {
    let required = root.get("required").and_then(Value::as_array);
    let mut fields: Vec<_> = root
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, value)| Field {
            name: name.clone(),
            required: required.is_some_and(|a| a.iter().any(|v| v.as_str() == Some(name))),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            schema: resolve(root, value).clone(),
        })
        .collect();
    fields.sort_by_key(|field| !field.required);
    fields
}
impl Field {
    pub fn kind(&self) -> &str {
        match self.schema.get("type") {
            Some(Value::String(kind)) => kind,
            Some(Value::Array(types)) => types
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")
                .unwrap_or("string"),
            _ => match self
                .schema
                .get("enum")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
            {
                Some(Value::Bool(_)) => "boolean",
                Some(Value::Number(number)) if number.is_i64() || number.is_u64() => "integer",
                Some(Value::Number(_)) => "number",
                _ => "string",
            },
        }
    }
    pub fn choices(&self) -> Vec<Value> {
        self.schema
            .get("enum")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
    pub fn default_text(&self) -> String {
        self.schema
            .get("default")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default()
    }
    pub fn parse(&self, raw: &str) -> Result<Value, String> {
        let invalid =
            || rust_i18n::t!("app.palette.invalid_field", field = self.name.clone()).into_owned();
        let value = match self.kind() {
            "boolean" => match raw {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return Err(invalid()),
            },
            "integer" => {
                let number = raw
                    .strip_prefix("0x")
                    .map(|s| i64::from_str_radix(s, 16))
                    .unwrap_or_else(|| raw.parse());
                Value::from(number.map_err(|_| invalid())?)
            }
            "number" => {
                let number = raw.parse::<f64>().map_err(|_| invalid())?;
                Value::Number(serde_json::Number::from_f64(number).ok_or_else(invalid)?)
            }
            "array" => {
                if raw.starts_with('[') {
                    let value: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
                    if !value.is_array() {
                        return Err(invalid());
                    }
                    value
                } else {
                    let item = Field {
                        name: self.name.clone(),
                        required: true,
                        description: String::new(),
                        schema: self
                            .schema
                            .get("items")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({"type":"string"})),
                    };
                    Value::Array(
                        raw.split(',')
                            .map(|s| item.parse(s.trim()))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                }
            }
            "object" => {
                let value: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
                if !value.is_object() {
                    return Err(invalid());
                }
                value
            }
            _ => Value::String(raw.to_owned()),
        };
        let choices = self.choices();
        if !choices.is_empty() && !choices.contains(&value) {
            return Err(invalid());
        }
        if let Some(number) = value.as_f64()
            && (self
                .schema
                .get("minimum")
                .and_then(Value::as_f64)
                .is_some_and(|min| number < min)
                || self
                    .schema
                    .get("maximum")
                    .and_then(Value::as_f64)
                    .is_some_and(|max| number > max))
        {
            return Err(invalid());
        }
        Ok(value)
    }
}
pub(super) fn build<'a>(
    fields: impl IntoIterator<Item = (&'a Field, String)>,
) -> Result<Value, String> {
    let mut params = Map::new();
    for (field, raw) in fields {
        if raw.is_empty() {
            if field.required {
                return Err(rust_i18n::t!(
                    "app.palette.required_field",
                    field = field.name.clone()
                )
                .into_owned());
            }
            continue;
        }
        params.insert(field.name.clone(), field.parse(&raw)?);
    }
    Ok(Value::Object(params))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn enum_array_boolean_and_integer_are_typed() {
        let fields = fields(
            &json!({"properties":{"ids":{"type":"array","items":{"type":"integer"}},"enabled":{"type":"boolean"},"direction":{"enum":["forward","backward"]}}}),
        );
        let value = build(fields.iter().map(|field| {
            (
                field,
                match field.name.as_str() {
                    "ids" => "1, 2",
                    "enabled" => "false",
                    _ => "backward",
                }
                .to_owned(),
            )
        }))
        .unwrap();
        assert_eq!(
            value,
            json!({"ids":[1,2],"enabled":false,"direction":"backward"})
        );
        assert!(
            fields
                .iter()
                .find(|f| f.name == "enabled")
                .unwrap()
                .parse("maybe")
                .is_err()
        );
    }
}
