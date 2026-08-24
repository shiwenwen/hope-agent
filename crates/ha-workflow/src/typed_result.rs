//! Bounded Workflow output-schema parsing and validation.

use anyhow::{anyhow, Context as _, Result};
use serde_json::Value;

const WORKFLOW_OUTPUT_SCHEMA_MAX_BYTES: usize = 16 * 1024;
const WORKFLOW_OUTPUT_SCHEMA_MAX_DEPTH: usize = 16;
const WORKFLOW_TYPED_RESULT_MAX_ERRORS: usize = 20;

pub fn workflow_output_schema(args: &Value) -> Result<Option<Value>> {
    let Some(schema) = args
        .get("outputSchema")
        .or_else(|| args.get("output_schema"))
    else {
        return Ok(None);
    };
    if !schema.is_object() {
        return Err(anyhow!(
            "workflow.spawnAgent outputSchema must be an object"
        ));
    }
    let encoded = serde_json::to_vec(schema)?;
    if encoded.len() > WORKFLOW_OUTPUT_SCHEMA_MAX_BYTES {
        return Err(anyhow!(
            "workflow.spawnAgent outputSchema exceeds {} bytes",
            WORKFLOW_OUTPUT_SCHEMA_MAX_BYTES
        ));
    }
    validate_workflow_schema_definition(schema, "$", 0)?;
    Ok(Some(schema.clone()))
}

fn validate_workflow_schema_definition(schema: &Value, path: &str, depth: usize) -> Result<()> {
    if depth > WORKFLOW_OUTPUT_SCHEMA_MAX_DEPTH {
        return Err(anyhow!(
            "workflow output schema exceeds max depth at {path}"
        ));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| anyhow!("workflow output schema at {path} must be an object"))?;
    const SUPPORTED: &[&str] = &[
        "$schema",
        "title",
        "description",
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "anyOf",
        "oneOf",
        "allOf",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
    ];
    for key in object.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            return Err(anyhow!(
                "unsupported workflow output schema keyword `{key}` at {path}"
            ));
        }
    }
    if let Some(kind) = object.get("type") {
        let valid = kind.as_str().is_some_and(is_supported_schema_type)
            || kind.as_array().is_some_and(|types| {
                !types.is_empty()
                    && types
                        .iter()
                        .all(|value| value.as_str().is_some_and(is_supported_schema_type))
            });
        if !valid {
            return Err(anyhow!("invalid workflow output schema type at {path}"));
        }
    }
    if let Some(properties) = object.get("properties") {
        let properties = properties
            .as_object()
            .ok_or_else(|| anyhow!("schema properties at {path} must be an object"))?;
        for (name, child) in properties {
            validate_workflow_schema_definition(
                child,
                &format!("{path}.properties.{name}"),
                depth + 1,
            )?;
        }
    }
    if let Some(required) = object.get("required") {
        if !required.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|value| !value.is_empty()))
        }) {
            return Err(anyhow!("schema required at {path} must be a string array"));
        }
    }
    if let Some(additional) = object.get("additionalProperties") {
        if !additional.is_boolean() {
            validate_workflow_schema_definition(
                additional,
                &format!("{path}.additionalProperties"),
                depth + 1,
            )?;
        }
    }
    if let Some(items) = object.get("items") {
        validate_workflow_schema_definition(items, &format!("{path}.items"), depth + 1)?;
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(branches) = object.get(keyword) {
            let branches = branches
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or_else(|| anyhow!("schema {keyword} at {path} must be a non-empty array"))?;
            for (index, branch) in branches.iter().enumerate() {
                validate_workflow_schema_definition(
                    branch,
                    &format!("{path}.{keyword}[{index}]"),
                    depth + 1,
                )?;
            }
        }
    }
    Ok(())
}

fn is_supported_schema_type(value: &str) -> bool {
    matches!(
        value,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    )
}

pub fn extract_workflow_typed_result(raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("<workflow_result>") {
        let content_start = start + "<workflow_result>".len();
        let end = trimmed[content_start..]
            .find("</workflow_result>")
            .map(|offset| content_start + offset)
            .ok_or_else(|| anyhow!("structured result is missing </workflow_result>"))?;
        return serde_json::from_str(trimmed[content_start..end].trim())
            .context("parse workflow_result JSON");
    }
    if trimmed.starts_with("```json") && trimmed.ends_with("```") {
        let body = trimmed
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();
        return serde_json::from_str(body).context("parse fenced structured result JSON");
    }
    serde_json::from_str(trimmed).context("parse structured result JSON")
}

pub fn validate_workflow_typed_value(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_workflow_typed_value_at(schema, value, "$", &mut errors);
    errors.truncate(WORKFLOW_TYPED_RESULT_MAX_ERRORS);
    errors
}

fn validate_workflow_typed_value_at(
    schema: &Value,
    value: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    if errors.len() >= WORKFLOW_TYPED_RESULT_MAX_ERRORS {
        return;
    }
    let Some(object) = schema.as_object() else {
        errors.push(format!("{path}: invalid schema"));
        return;
    };
    if let Some(expected) = object.get("const") {
        if value != expected {
            errors.push(format!("{path}: value does not match const"));
        }
    }
    if let Some(allowed) = object.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            errors.push(format!("{path}: value is not in enum"));
        }
    }
    if let Some(types) = object.get("type") {
        let matches = types
            .as_str()
            .is_some_and(|kind| workflow_value_matches_type(value, kind))
            || types.as_array().is_some_and(|items| {
                items.iter().any(|kind| {
                    kind.as_str()
                        .is_some_and(|kind| workflow_value_matches_type(value, kind))
                })
            });
        if !matches {
            errors.push(format!(
                "{path}: expected type {types}, got {}",
                workflow_value_type(value)
            ));
            return;
        }
    }
    if let Some(branches) = object.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            validate_workflow_typed_value_at(branch, value, path, errors);
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
            let matches = branches
                .iter()
                .filter(|branch| validate_workflow_typed_value(branch, value).is_empty())
                .count();
            let valid = if keyword == "oneOf" {
                matches == 1
            } else {
                matches >= 1
            };
            if !valid {
                errors.push(format!("{path}: {keyword} matched {matches} branches"));
            }
        }
    }
    if let Some(map) = value.as_object() {
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !map.contains_key(key) {
                    errors.push(format!("{path}.{key}: required property is missing"));
                }
            }
        }
        if let Some(properties) = properties {
            for (key, child_schema) in properties {
                if let Some(child) = map.get(key) {
                    validate_workflow_typed_value_at(
                        child_schema,
                        child,
                        &format!("{path}.{key}"),
                        errors,
                    );
                }
            }
        }
        if let Some(additional) = object.get("additionalProperties") {
            for (key, child) in map {
                if properties.is_some_and(|properties| properties.contains_key(key)) {
                    continue;
                }
                if additional == &Value::Bool(false) {
                    errors.push(format!("{path}.{key}: additional property is not allowed"));
                } else if additional.is_object() {
                    validate_workflow_typed_value_at(
                        additional,
                        child,
                        &format!("{path}.{key}"),
                        errors,
                    );
                }
            }
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(min) = object.get("minItems").and_then(Value::as_u64) {
            if items.len() < min as usize {
                errors.push(format!("{path}: expected at least {min} items"));
            }
        }
        if let Some(max) = object.get("maxItems").and_then(Value::as_u64) {
            if items.len() > max as usize {
                errors.push(format!("{path}: expected at most {max} items"));
            }
        }
        if let Some(item_schema) = object.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_workflow_typed_value_at(
                    item_schema,
                    item,
                    &format!("{path}[{index}]"),
                    errors,
                );
            }
        }
    }
    if let Some(text) = value.as_str() {
        if let Some(min) = object.get("minLength").and_then(Value::as_u64) {
            if text.chars().count() < min as usize {
                errors.push(format!("{path}: string is shorter than {min}"));
            }
        }
        if let Some(max) = object.get("maxLength").and_then(Value::as_u64) {
            if text.chars().count() > max as usize {
                errors.push(format!("{path}: string is longer than {max}"));
            }
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(min) = object.get("minimum").and_then(Value::as_f64) {
            if number < min {
                errors.push(format!("{path}: number is below minimum {min}"));
            }
        }
        if let Some(max) = object.get("maximum").and_then(Value::as_f64) {
            if number > max {
                errors.push(format!("{path}: number is above maximum {max}"));
            }
        }
    }
}

fn workflow_value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn workflow_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
