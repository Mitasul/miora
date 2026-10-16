use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigField {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default, rename = "enum")]
    pub choices: Vec<Value>,
    #[serde(default)]
    pub secret: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Authentication {
    #[serde(default = "none", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub config_key: String,
    #[serde(default)]
    pub header: String,
}
impl Default for Authentication {
    fn default() -> Self {
        Self {
            kind: "none".into(),
            config_key: String::new(),
            header: String::new(),
        }
    }
}
fn none() -> String {
    "none".into()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capabilities {
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub model_discovery: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub config_schema: Vec<ConfigField>,
    #[serde(default)]
    pub auth: Authentication,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub models: Vec<Model>,
}
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) || self.id.starts_with("builtin-") {
            return Err("invalid or reserved plugin id".into());
        }
        if self.api_version != 1 {
            return Err("unsupported plugin api_version".into());
        }
        if self.name.trim().is_empty()
            || self.version.trim().is_empty()
            || self.name.len() > 128
            || self.version.len() > 64
            || self.description.len() > 4096
        {
            return Err("invalid manifest metadata".into());
        }
        let supported = [
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/responses/compact",
            "/v1/embeddings",
            "/v1/images/generations",
            "/v1/audio/speech",
            "/v1/moderations",
            "/v1/rerank",
            "/v1/search",
            "/search",
            "/v1/alpha/search",
        ];
        if self.capabilities.endpoints.is_empty()
            || self
                .capabilities
                .endpoints
                .iter()
                .any(|e| !supported.contains(&e.as_str()))
        {
            return Err(
                "unsupported or missing plugin endpoints (v1 accepts JSON requests only)".into(),
            );
        }
        if self.config_schema.len() > 128 || self.models.len() > 4096 {
            return Err("manifest size limit exceeded".into());
        }
        let mut keys = BTreeSet::new();
        for field in &self.config_schema {
            if !valid_id(&field.key)
                || !keys.insert(&field.key)
                || !["string", "integer", "number", "boolean", "object", "array"]
                    .contains(&field.kind.as_str())
            {
                return Err("invalid or duplicate config field".into());
            }
            if field.secret && (field.default.is_some() || !field.choices.is_empty()) {
                return Err("secret config fields cannot contain manifest defaults".into());
            }
            if let Some(value) = &field.default {
                field.validate(value)?;
            }
            if field.choices.iter().any(|v| !field.matches(v)) {
                return Err("enum value does not match field type".into());
            }
        }
        let kind = if self.auth.kind.is_empty() {
            "none"
        } else {
            &self.auth.kind
        };
        if !["none", "bearer", "api_key"].contains(&kind) {
            return Err("unsupported channel authentication".into());
        }
        if kind != "none"
            && !self
                .config_schema
                .iter()
                .any(|f| f.key == self.auth.config_key && f.secret && f.kind == "string")
        {
            return Err("authentication config_key must reference a secret string field".into());
        }
        if kind == "api_key"
            && (self.auth.header.is_empty()
                || self.auth.header.parse::<axum::http::HeaderName>().is_err())
        {
            return Err("invalid upstream authentication header declaration".into());
        }
        if self
            .models
            .iter()
            .any(|m| m.id.is_empty() || m.id.len() > 256 || m.name.len() > 256)
        {
            return Err("invalid model declaration".into());
        }
        Ok(())
    }
    pub fn config(
        &self,
        input: &Map<String, Value>,
        require: bool,
    ) -> Result<Map<String, Value>, String> {
        let mut result = Map::new();
        if input
            .keys()
            .any(|k| !self.config_schema.iter().any(|f| &f.key == k))
        {
            return Err("unknown plugin configuration field".into());
        }
        for field in &self.config_schema {
            match input.get(&field.key).or(field.default.as_ref()) {
                Some(v) => {
                    field.validate(v)?;
                    result.insert(field.key.clone(), v.clone());
                }
                None if require && field.required => {
                    return Err(format!("missing configuration field: {}", field.key))
                }
                None => {}
            }
        }
        Ok(result)
    }
    pub fn redact(&self, input: &Map<String, Value>) -> Map<String, Value> {
        input
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    if self
                        .config_schema
                        .iter()
                        .find(|f| f.key == *key)
                        .map_or(true, |f| f.secret)
                    {
                        Value::String("***".into())
                    } else {
                        value.clone()
                    },
                )
            })
            .collect()
    }
}
impl ConfigField {
    fn matches(&self, value: &Value) -> bool {
        match self.kind.as_str() {
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            _ => false,
        }
    }
    fn validate(&self, value: &Value) -> Result<(), String> {
        if (self.secret && value.as_str() == Some("***"))
            || !self.matches(value)
            || (!self.choices.is_empty() && !self.choices.contains(value))
        {
            Err(format!("invalid configuration field: {}", self.key))
        } else {
            Ok(())
        }
    }
}
