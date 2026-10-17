use super::{header_sensitive, profile, service, PolicyReport, Profile};
use crate::config::snapshot::Provider;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Map, Value};

const BUDGETS: [&str; 4] = [
    "/max_tokens",
    "/max_completion_tokens",
    "/max_output_tokens",
    "/generationConfig/maxOutputTokens",
];
const GENERATION: [&str; 10] = [
    "/temperature",
    "/top_p",
    "/max_tokens",
    "/max_completion_tokens",
    "/max_output_tokens",
    "/stop",
    "/stop_sequences",
    "/generationConfig/temperature",
    "/generationConfig/topP",
    "/generationConfig/maxOutputTokens",
];
const MODES: [&str; 5] = ["inherit", "prepend", "append", "replace", "remove"];

pub(crate) struct BodyOrigin {
    client: Value,
    constructed: Value,
}
pub(crate) fn exclude_plugin_fields<'a>(
    origin: &mut BodyOrigin,
    keys: impl Iterator<Item = &'a String>,
) {
    if let Some(root) = origin.client.as_object_mut() {
        for key in keys {
            root.remove(key);
        }
    }
}
pub(crate) fn capture_origin(
    provider: &Provider,
    client: &Value,
    constructed: &Value,
) -> Option<BodyOrigin> {
    provider
        .preferences
        .contains_key("parameter_policy")
        .then(|| BodyOrigin {
            client: client.clone(),
            constructed: constructed.clone(),
        })
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{path}: expected an object"))
}
fn enabled(row: &Value) -> bool {
    row.get("enabled").and_then(Value::as_bool).unwrap_or(true)
}
fn pointer_parts(path: &str) -> Result<Vec<String>, String> {
    if !path.starts_with('/') {
        return Err("remove: expected a non-root JSON pointer".into());
    }
    let mut parts = Vec::new();
    for part in path[1..].split('/') {
        let mut decoded = String::new();
        let mut chars = part.chars();
        while let Some(ch) = chars.next() {
            if ch != '~' {
                decoded.push(ch);
            } else {
                decoded.push(match chars.next() {
                    Some('0') => '~',
                    Some('1') => '/',
                    _ => return Err("remove: invalid JSON pointer escape".into()),
                });
            }
        }
        parts.push(decoded);
    }
    if parts.len() > 32 {
        return Err("remove: JSON pointer nesting exceeds limit".into());
    }
    Ok(parts)
}
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
fn protected_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "upgrade",
        "trailer",
        "te",
        "proxy-authorization",
        "proxy-authenticate",
        "x-request-id",
        "x-caller-request-id",
    ]
    .contains(&name.as_str())
        || ["x-miora-", "x-uni-api-", "x-oaix-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}
fn validate_section(section: &Value, label: &str) -> Result<(), String> {
    let section = object(section, label)?;
    if let Some(set) = section.get("set") {
        object(set, &format!("{label}.set"))?;
    }
    if let Some(remove) = section.get("remove") {
        let paths = remove
            .as_array()
            .ok_or_else(|| format!("{label}.remove: expected an array"))?;
        if paths.len() > 256 {
            return Err(format!("{label}.remove: too many entries"));
        }
        for path in paths {
            let parts = pointer_parts(
                path.as_str()
                    .ok_or_else(|| format!("{label}.remove: entries must be strings"))?,
            )?;
            if matches!(parts[0].as_str(), "model" | "stream") {
                return Err(format!(
                    "{label}.remove: model/stream are routing-controlled"
                ));
            }
        }
    }
    if section
        .get("set")
        .and_then(Value::as_object)
        .is_some_and(|set| set.contains_key("model") || set.contains_key("stream"))
    {
        return Err(format!("{label}.set: model/stream are routing-controlled"));
    }
    Ok(())
}

/// Structural validation only; unsupported fields for a concrete wire protocol
/// are checked by the shared production prepare. Unknown extension keys survive.
pub(crate) fn validate_policy(value: &Value) -> Result<(), String> {
    let root = object(value, "parameter_policy")?;
    if let Some(defaults) = root.get("defaults") {
        let defaults = object(defaults, "defaults")?;
        if defaults.contains_key("model") || defaults.contains_key("stream") {
            return Err("defaults: model/stream are routing-controlled".into());
        }
    }
    if let Some(channel) = root.get("channel") {
        validate_section(channel, "channel")?;
    }
    if let Some(models) = root.get("models") {
        let models = models.as_array().ok_or("models: expected an array")?;
        if models.len() > 128 {
            return Err("models: too many rules".into());
        }
        for (index, rule) in models.iter().enumerate() {
            let label = format!("models[{index}]");
            validate_section(rule, &label)?;
            let name = rule
                .get("match")
                .and_then(|m| m.get("requested_model"))
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{label}.match.requested_model: expected a string"))?;
            if name.is_empty() || name.len() > 512 {
                return Err(format!("{label}.match.requested_model: invalid length"));
            }
        }
    }
    if let Some(headers) = root.get("headers") {
        let headers = headers.as_array().ok_or("headers: expected an array")?;
        if headers.len() > 64 {
            return Err("headers: too many rules".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for (index, row) in headers.iter().enumerate() {
            let label = format!("headers[{index}]");
            let row = object(row, &label)?;
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{label}.name: expected a string"))?;
            name.parse::<HeaderName>()
                .map_err(|_| format!("{label}.name: invalid HTTP header name"))?;
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(format!(
                    "{label}.name: duplicate header names are ambiguous"
                ));
            }
            for flag in ["enabled", "secret"] {
                if row.get(flag).is_some_and(|v| !v.is_boolean()) {
                    return Err(format!("{label}.{flag}: expected a boolean"));
                }
            }
            let mode = row
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{label}.mode: expected inherit/set/remove"))?;
            if !["inherit", "set", "remove"].contains(&mode) {
                return Err(format!("{label}.mode: expected inherit/set/remove"));
            }
            if row.get("enabled").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            if mode != "inherit" && protected_header(name) {
                return Err(format!(
                    "{label}.name: transport/internal header is protected"
                ));
            }
            if mode == "set" {
                let value=row.get("value").and_then(Value::as_str).ok_or_else(||format!("{label}.value: set requires a string, including an explicitly empty string"))?;
                value
                    .parse::<HeaderValue>()
                    .map_err(|_| format!("{label}.value: invalid HTTP header value"))?;
            }
        }
    }
    if let Some(system) = root.get("system_prompt") {
        let system = object(system, "system_prompt")?;
        let mode = system
            .get("mode")
            .and_then(Value::as_str)
            .ok_or("system_prompt.mode: expected a string")?;
        if !MODES.contains(&mode) {
            return Err("system_prompt.mode: unsupported mode".into());
        }
        if matches!(mode, "prepend" | "append" | "replace")
            && !system.get("value").is_some_and(Value::is_string)
        {
            return Err(
                "system_prompt.value: expected a string, including an explicitly empty string"
                    .into(),
            );
        }
    }
    Ok(())
}

fn set_pointer(value: &mut Value, path: &str, replacement: Value) -> Result<(), String> {
    let parts = pointer_parts(path)?;
    let mut current = value;
    for part in &parts[..parts.len() - 1] {
        if !current.is_object() {
            *current = json!({});
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry(part.clone())
            .or_insert_with(|| json!({}));
    }
    if !current.is_object() {
        *current = json!({});
    }
    current
        .as_object_mut()
        .unwrap()
        .insert(parts.last().unwrap().clone(), replacement);
    Ok(())
}
fn remove_pointer(value: &mut Value, path: &str) -> Result<(), String> {
    let parts = pointer_parts(path)?;
    let mut current = value;
    for part in &parts[..parts.len() - 1] {
        current = match current {
            Value::Object(map) => match map.get_mut(part) {
                Some(value) => value,
                None => return Ok(()),
            },
            Value::Array(items) => match part
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get_mut(index))
            {
                Some(value) => value,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
    }
    let last = parts.last().unwrap();
    match current {
        Value::Object(map) => {
            map.remove(last);
        }
        Value::Array(items) => {
            let index = last
                .parse::<usize>()
                .map_err(|_| "remove: invalid array index")?;
            if index < items.len() {
                items.remove(index);
            }
        }
        _ => {}
    }
    Ok(())
}
fn merge(target: &mut Value, patch: &Value, path: &str, source: &str, report: &mut PolicyReport) {
    if let Value::Object(patch) = patch {
        if !target.is_object() {
            *target = json!({});
        }
        for (key, value) in patch {
            merge(
                target
                    .as_object_mut()
                    .unwrap()
                    .entry(key.clone())
                    .or_insert(Value::Null),
                value,
                &format!("{path}/{}", escape(key)),
                source,
                report,
            );
        }
    } else {
        *target = patch.clone();
        report.record(path, source, "set");
    }
}
fn default_can_write(patch: &Value, presence: Option<&Value>) -> bool {
    match patch {
        Value::Object(map) if presence.map_or(true, Value::is_object) => {
            (map.is_empty() && presence.is_none())
                || map
                    .iter()
                    .any(|(key, value)| default_can_write(value, presence.and_then(|p| p.get(key))))
        }
        _ => presence.is_none(),
    }
}
fn defaults(
    target: &mut Value,
    patch: &Value,
    presence: Option<&Value>,
    path: &str,
    report: &mut PolicyReport,
) {
    match patch {
        Value::Object(patch) if presence.map_or(true, Value::is_object) => {
            if !target.is_object() {
                *target = json!({});
            }
            for (key, value) in patch {
                let child_presence = presence.and_then(|p| p.get(key));
                let child_path = format!("{path}/{}", escape(key));
                if !default_can_write(value, child_presence) {
                    // Never create a null placeholder when an adapter/legacy
                    // layer deliberately omitted a client-provided field.
                    report.record(&child_path, "client", "retain");
                    continue;
                }
                defaults(
                    target
                        .as_object_mut()
                        .unwrap()
                        .entry(key.clone())
                        .or_insert(Value::Null),
                    value,
                    child_presence,
                    &child_path,
                    report,
                );
            }
        }
        _ if presence.is_some() => report.record(path, "client", "retain"),
        _ => {
            *target = patch.clone();
            report.record(path, "defaults", "set");
        }
    }
}
fn body_active(policy: &Value, request_model: &str) -> bool {
    policy
        .get("defaults")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty())
        || policy.get("channel").is_some_and(section_active)
        || policy
            .get("models")
            .and_then(Value::as_array)
            .is_some_and(|rules| {
                rules.iter().any(|r| {
                    r.pointer("/match/requested_model").and_then(Value::as_str)
                        == Some(request_model)
                        && section_active(r)
                })
            })
        || policy
            .pointer("/system_prompt/mode")
            .and_then(Value::as_str)
            .is_some_and(|m| m != "inherit")
}
fn section_active(value: &Value) -> bool {
    value
        .get("set")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty())
        || value
            .get("remove")
            .and_then(Value::as_array)
            .is_some_and(|m| !m.is_empty())
}
fn legacy_controls(provider: &Provider, request_model: &str, path: &str) -> bool {
    let top = path.trim_start_matches('/').split('/').next().unwrap_or("");
    let Some(root) = provider
        .preferences
        .get("post_body_parameter_overrides")
        .and_then(Value::as_object)
    else {
        return false;
    };
    root.contains_key(top)
        || root
            .get(request_model)
            .and_then(Value::as_object)
            .is_some_and(|m| m.contains_key(top))
        || root.contains_key("__remove__")
        || root
            .get(request_model)
            .and_then(Value::as_object)
            .is_some_and(|m| m.contains_key("__remove__"))
}
fn client_presence(
    provider: &Provider,
    request_model: &str,
    profile: Profile,
    origin: &BodyOrigin,
    body: &mut Value,
    report: &mut PolicyReport,
) -> Result<Value, String> {
    let mut presence = origin.client.clone();
    if !presence.is_object() {
        presence = json!({});
    }
    let families: [(Option<&str>, &[&str]); 4] = [
        (
            profile.temperature(),
            &["/temperature", "/generationConfig/temperature"],
        ),
        (profile.top_p(), &["/top_p", "/generationConfig/topP"]),
        (profile.budget(), &BUDGETS),
        (
            profile.stop(),
            &[
                "/stop",
                "/stop_sequences",
                "/generationConfig/stopSequences",
            ],
        ),
    ];
    for (target, aliases) in families {
        let Some(mut target) = target else {
            continue;
        };
        let is_budget = Some(target) == profile.budget();
        if is_budget
            && profile == Profile::Chat
            && (origin.client.pointer("/max_completion_tokens").is_some()
                || origin
                    .constructed
                    .pointer("/max_completion_tokens")
                    .is_some())
        {
            // Preserve a valid Chat completion-token field; an unrelated policy
            // must not downgrade o-series requests to deprecated max_tokens.
            target = "/max_completion_tokens";
        }
        let source = origin
            .client
            .pointer(target)
            .map(|v| (target, v))
            .or_else(|| {
                aliases
                    .iter()
                    .find_map(|path| origin.client.pointer(path).map(|v| (*path, v)))
            });
        if let Some((source, value)) = source {
            let value = if Some(target) == profile.stop()
                && profile != Profile::Chat
                && value.is_string()
            {
                json!([value])
            } else {
                value.clone()
            };
            set_pointer(&mut presence, target, value.clone())?;
            // Presence belongs to the semantic budget, not only one spelling.
            // A default under another alias cannot override a supplied budget.
            if is_budget {
                for alias in BUDGETS {
                    set_pointer(&mut presence, alias, value.clone())?;
                }
            }
            // A client value is not an adapter-generated fallback. Preserve any
            // explicit legacy override; otherwise map aliases the adapter did
            // not understand, rather than letting defaults replace the client.
            let legacy_budget = is_budget
                && BUDGETS
                    .iter()
                    .any(|p| legacy_controls(provider, request_model, p));
            if (source != target || body.pointer(target).is_none())
                && !legacy_budget
                && !legacy_controls(provider, request_model, target)
                && body.pointer(target) == origin.constructed.pointer(target)
            {
                set_pointer(body, target, value)?;
                report.record(target, "client_mapping", "set");
                if is_budget {
                    for other in BUDGETS {
                        if other != target {
                            remove_pointer(body, other)?;
                        }
                    }
                }
            }
        }
    }
    if let Some(system) = profile.system() {
        let has_system = origin
            .client
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|r| r.get("role").and_then(Value::as_str) == Some("system"))
            })
            || ["instructions", "system", "systemInstruction"]
                .iter()
                .any(|key| origin.client.get(key).is_some());
        if has_system && origin.client.pointer(system).is_none() {
            set_pointer(&mut presence, system, Value::Null)?;
        }
    }
    Ok(presence)
}
fn validate_patch(profile: Profile, patch: &Value) -> Result<(), String> {
    for name in ["reasoning", "reasoning_effort", "thinking", "service_tier"] {
        if patch.get(name).is_some() {
            return Err(format!(
                "parameter_policy: {name} capability is unsupported"
            ));
        }
    }
    let selected = BUDGETS
        .iter()
        .filter(|path| patch.pointer(path).is_some())
        .count();
    if selected > 1 {
        return Err("parameter_policy: declare exactly one output budget field per section".into());
    }
    for path in GENERATION
        .into_iter()
        .chain(["/generationConfig/stopSequences"])
    {
        if patch.pointer(path).is_none() {
            continue;
        }
        let allowed = [
            profile.temperature(),
            profile.top_p(),
            profile.budget(),
            profile.stop(),
        ]
        .contains(&Some(path))
            || (profile == Profile::Chat && path == "/max_completion_tokens");
        if !allowed {
            return Err(format!(
                "parameter_policy: field {path} is unsupported for this wire profile"
            ));
        }
    }
    Ok(())
}
fn consolidate_budget(
    body: &mut Value,
    selected: &str,
    report: &mut PolicyReport,
) -> Result<(), String> {
    for other in BUDGETS {
        if other != selected && body.pointer(other).is_some() {
            remove_pointer(body, other)?;
            report.record(other, "protocol_constraint", "remove");
        }
    }
    Ok(())
}
fn apply_section(
    body: &mut Value,
    section: &Value,
    profile: Profile,
    source: &str,
    report: &mut PolicyReport,
) -> Result<(), String> {
    if let Some(patch) = section.get("set") {
        validate_patch(profile, patch)?;
        merge(body, patch, "", source, report);
        if let Some(selected) = BUDGETS.iter().find(|path| patch.pointer(path).is_some()) {
            consolidate_budget(body, selected, report)?;
        }
    }
    if let Some(remove) = section.get("remove").and_then(Value::as_array) {
        for pointer in remove {
            let path = pointer.as_str().unwrap();
            remove_pointer(body, path)?;
            report.record(path, source, "remove");
        }
    }
    Ok(())
}
fn strings(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}
fn constraints(profile: Profile, body: &Value) -> Result<(), String> {
    if profile == Profile::Claude && body.get("max_tokens").is_none() {
        return Err("parameter_policy: Claude max_tokens is required and cannot be removed".into());
    }
    for (path, max) in [
        (
            profile.temperature(),
            if profile == Profile::Claude { 1.0 } else { 2.0 },
        ),
        (profile.top_p(), 1.0),
    ] {
        if let Some((path, value)) = path.and_then(|path| body.pointer(path).map(|v| (path, v))) {
            if value.is_null() && matches!(profile, Profile::Chat | Profile::Responses) {
                continue;
            }
            if !value.as_f64().is_some_and(|v| v >= 0.0 && v <= max) {
                return Err(format!(
                    "parameter_policy: {path} is outside the supported numeric range"
                ));
            }
        }
    }
    for path in BUDGETS {
        if let Some(value) = body.pointer(path) {
            if value.as_u64().is_none_or(|v| v == 0) {
                return Err(format!(
                    "parameter_policy: {path} requires a positive integer"
                ));
            }
        }
    }
    if BUDGETS
        .iter()
        .filter(|path| body.pointer(path).is_some())
        .count()
        > 1
    {
        return Err("parameter_policy: conflicting output budget fields remain".into());
    }
    if let Some((path, value)) = profile
        .stop()
        .and_then(|path| body.pointer(path).map(|v| (path, v)))
    {
        let valid =
            strings(value) || (profile == Profile::Chat && (value.is_string() || value.is_null()));
        if !valid {
            return Err(format!(
                "parameter_policy: {path} has an unsupported stop-sequence type"
            ));
        }
    }
    if profile == Profile::Claude && body.get("thinking").is_some() {
        if body
            .get("temperature")
            .is_some_and(|v| v.as_f64() != Some(1.0))
            || body.get("top_p").is_some()
        {
            return Err("parameter_policy: sampling policy conflicts with existing Claude thinking constraints".into());
        }
        if let Some(budget) = body
            .pointer("/thinking/budget_tokens")
            .and_then(Value::as_u64)
        {
            if body
                .get("max_tokens")
                .and_then(Value::as_u64)
                .map_or(true, |v| v <= budget)
            {
                return Err(
                    "parameter_policy: output budget must exceed existing thinking budget".into(),
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn apply_body(
    provider: &Provider,
    request_model: &str,
    path: &str,
    origin: Option<&BodyOrigin>,
    body: &mut Value,
) -> Result<PolicyReport, String> {
    let mut report = PolicyReport::new(provider, path);
    crate::providers::reliability::apply(provider, request_model, path, body)?;
    let Some(policy) = provider.preferences.get("parameter_policy") else {
        return Ok(report);
    };
    validate_policy(policy)?;
    if policy.as_object().unwrap().keys().any(|key| {
        !["defaults", "channel", "models", "headers", "system_prompt"].contains(&key.as_str())
    }) {
        report
            .warnings
            .push("Unknown policy extensions are preserved but not executed.".into());
    }
    if service(provider)
        && policy
            .get("headers")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|r| enabled(r) && r.get("mode").and_then(Value::as_str) != Some("inherit"))
            })
    {
        return Err(
            "parameter_policy.headers: service control-channel headers cannot be overridden".into(),
        );
    }
    if !body_active(policy, request_model) {
        return Ok(report);
    }
    let profile = profile(provider, path);
    if profile == Profile::Unsupported {
        report.record("/body", "unsupported_endpoint_or_profile", "skipped");
        report.warnings.push(
            "Body parameter policy is not applied for this endpoint/provider profile.".into(),
        );
        return Ok(report);
    }
    let origin=origin.ok_or("parameter_policy: body policy is unsupported for replay/multipart input without client provenance")?;
    let mut candidate = body.clone();
    let presence = client_presence(
        provider,
        request_model,
        profile,
        origin,
        &mut candidate,
        &mut report,
    )?;
    if let Some(patch) = policy.get("defaults") {
        validate_patch(profile, patch)?;
        defaults(&mut candidate, patch, Some(&presence), "", &mut report);
        if let Some(selected) = BUDGETS.iter().find(|path| patch.pointer(path).is_some()) {
            if report
                .trace
                .iter()
                .any(|t| t.source == "defaults" && t.path == *selected && t.action == "set")
            {
                consolidate_budget(&mut candidate, selected, &mut report)?;
            }
        }
    }
    if let Some(section) = policy.get("channel") {
        apply_section(&mut candidate, section, profile, "channel", &mut report)?;
    }
    // system_prompt is channel-level. Exact requested-model rules must retain
    // the final say over instructions/messages before protocol constraints.
    if let Some(system) = policy.get("system_prompt") {
        apply_system(&mut candidate, profile, system, &origin.client, &mut report)?;
    }
    if let Some(rules) = policy.get("models").and_then(Value::as_array) {
        for (index, rule) in rules.iter().enumerate() {
            if rule
                .pointer("/match/requested_model")
                .and_then(Value::as_str)
                == Some(request_model)
            {
                apply_section(
                    &mut candidate,
                    rule,
                    profile,
                    &format!("models[{index}]"),
                    &mut report,
                )?;
            }
        }
    }
    constraints(profile, &candidate)?;
    *body = candidate;
    Ok(report)
}

fn joined(before: &str, after: &str) -> String {
    if before.is_empty() {
        after.into()
    } else if after.is_empty() {
        before.into()
    } else {
        format!("{before}\n{after}")
    }
}
fn text(content: &Value) -> Result<String, String> {
    if let Some(text) = content.as_str() {
        return Ok(text.into());
    }
    let parts = content
        .as_array()
        .ok_or("system_prompt: cannot convert non-text system content")?;
    parts
        .iter()
        .map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or("system_prompt: cannot convert non-text system content".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n"))
}
fn apply_system(
    body: &mut Value,
    profile: Profile,
    policy: &Value,
    client: &Value,
    report: &mut PolicyReport,
) -> Result<(), String> {
    let mode = policy.get("mode").and_then(Value::as_str).unwrap();
    if mode == "inherit" {
        return Ok(());
    }
    let value = policy.get("value").and_then(Value::as_str).unwrap_or("");
    let path = profile
        .system()
        .ok_or("system_prompt: unsupported profile")?;
    // Chat-to-Responses carries original system messages in input. Lift only
    // those system entries for this explicit prompt operation, never developers
    // or ordinary user messages. Native Responses operates on instructions only.
    if matches!(profile, Profile::Responses | Profile::Codex) && client.get("messages").is_some() {
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            let mut parts = Vec::new();
            for item in input
                .iter()
                .filter(|item| item.get("role").and_then(Value::as_str) == Some("system"))
            {
                if !matches!(mode, "replace" | "remove") {
                    parts.push(text(item.get("content").unwrap_or(&Value::Null))?);
                }
            }
            input.retain(|item| item.get("role").and_then(Value::as_str) != Some("system"));
            if !parts.is_empty() {
                let old = body
                    .get("instructions")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                set_pointer(body, "/instructions", json!(joined(&parts.join("\n"), old)))?;
            }
        }
    }
    match profile {
        Profile::Chat => {
            let root = body
                .as_object_mut()
                .ok_or("system_prompt: body must be an object")?;
            let messages = root
                .entry("messages")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or("system_prompt: messages must be an array")?;
            let first = messages
                .iter()
                .position(|m| m.get("role").and_then(Value::as_str) == Some("system"));
            let last = messages
                .iter()
                .rposition(|m| m.get("role").and_then(Value::as_str) == Some("system"));
            if matches!(mode, "replace" | "remove") {
                messages.retain(|m| m.get("role").and_then(Value::as_str) != Some("system"));
            }
            if mode != "remove" {
                let index = if mode == "append" {
                    last.map_or(0, |i| i + 1)
                } else {
                    first.unwrap_or(0)
                };
                messages.insert(
                    index.min(messages.len()),
                    json!({"role":"system","content":value}),
                );
            }
        }
        Profile::Responses | Profile::Codex | Profile::Claude => {
            if mode == "remove" {
                remove_pointer(body, path)?;
            } else if mode == "replace" || body.pointer(path).is_none() {
                set_pointer(body, path, json!(value))?;
            } else if profile == Profile::Claude && body.pointer(path).is_some_and(Value::is_array)
            {
                let blocks = body.pointer_mut(path).unwrap().as_array_mut().unwrap();
                let block = json!({"type":"text","text":value});
                if mode == "prepend" {
                    blocks.insert(0, block);
                } else {
                    blocks.push(block);
                }
            } else {
                let old = body.pointer(path).and_then(Value::as_str).ok_or(
                    "system_prompt: existing prompt must be text or supported text blocks",
                )?;
                let combined = if mode == "prepend" {
                    joined(value, old)
                } else {
                    joined(old, value)
                };
                set_pointer(body, path, json!(combined))?;
            }
        }
        Profile::Gemini => {
            if mode == "remove" {
                remove_pointer(body, path)?;
            } else if mode == "replace" || body.pointer(path).is_none() {
                set_pointer(body, path, json!({"parts":[{"text":value}]}))?;
            } else {
                let parts = body
                    .pointer_mut("/systemInstruction/parts")
                    .and_then(Value::as_array_mut)
                    .ok_or("system_prompt: systemInstruction.parts must be an array")?;
                if mode == "prepend" {
                    parts.insert(0, json!({"text":value}));
                } else {
                    parts.push(json!({"text":value}));
                }
            }
        }
        Profile::Unsupported => return Err("system_prompt: unsupported profile".into()),
    }
    report.record(
        path,
        "system_prompt",
        if mode == "remove" { "remove" } else { "set" },
    );
    Ok(())
}

pub(crate) fn apply_headers(
    provider: &Provider,
    path: &str,
    headers: &mut HeaderMap,
    report: &mut PolicyReport,
) -> Result<(), String> {
    let Some(policy) = provider.preferences.get("parameter_policy") else {
        return Ok(());
    };
    let mut candidate = headers.clone();
    if let Some(rows) = policy.get("headers").and_then(Value::as_array) {
        for (index, row) in rows.iter().enumerate() {
            if !enabled(row) {
                continue;
            }
            let name = row.get("name").and_then(Value::as_str).unwrap();
            let mode = row.get("mode").and_then(Value::as_str).unwrap();
            let secret = header_sensitive(
                name,
                row.get("secret").and_then(Value::as_bool).unwrap_or(false),
            );
            if secret {
                report.sensitive_headers.push(name.to_ascii_lowercase());
            }
            if mode == "inherit" {
                report.record(
                    &format!("/headers/{}", escape(&name.to_ascii_lowercase())),
                    &format!("headers[{index}]"),
                    "retain",
                );
                continue;
            }
            if service(provider) {
                return Err("parameter_policy.headers: service control-channel headers cannot be overridden".into());
            }
            let engine = provider.engine.to_ascii_lowercase();
            if (matches!(
                engine.as_str(),
                "vertex" | "vertex-gemini" | "vertex-claude" | "aws"
            ) && name.eq_ignore_ascii_case("authorization"))
                || (engine == "aws" && name.to_ascii_lowercase().starts_with("x-amz-"))
            {
                return Err(format!(
                    "headers[{index}]: authentication header is owned by the dispatch signer"
                ));
            }
            let parsed = name
                .parse::<HeaderName>()
                .map_err(|_| "headers: invalid name")?;
            if mode == "remove" {
                candidate.remove(&parsed);
            } else {
                candidate.insert(
                    parsed,
                    row.get("value")
                        .and_then(Value::as_str)
                        .unwrap()
                        .parse::<HeaderValue>()
                        .map_err(|_| "headers: invalid value")?,
                );
            }
            report.record(
                &format!("/headers/{}", escape(&name.to_ascii_lowercase())),
                &format!("headers[{index}]"),
                mode,
            );
        }
    }
    report.header_supported = !service(provider);
    if report.profile.is_empty() {
        *report = PolicyReport::new(provider, path);
    }
    for name in candidate.keys() {
        if header_sensitive(name.as_str(), false) {
            report.sensitive_headers.push(name.to_string());
        }
    }
    report.sensitive_headers.sort();
    report.sensitive_headers.dedup();
    *headers = candidate;
    Ok(())
}
pub(crate) fn apply_native_headers(
    provider: &Provider,
    path: &str,
    headers: &mut std::collections::HashMap<String, String>,
    report: &mut PolicyReport,
) -> Result<(), String> {
    if !provider
        .preferences
        .get("parameter_policy")
        .and_then(|policy| policy.get("headers"))
        .and_then(Value::as_array)
        .is_some_and(|rows| rows.iter().any(enabled))
    {
        return Ok(());
    }
    let mut typed = HeaderMap::new();
    for (name, value) in headers.iter() {
        typed.insert(
            name.parse::<HeaderName>()
                .map_err(|_| "headers: invalid name")?,
            value
                .parse::<HeaderValue>()
                .map_err(|_| "headers: invalid value")?,
        );
    }
    apply_headers(provider, path, &mut typed, report)?;
    *headers = typed
        .iter()
        .map(|(name, value)| {
            Ok((
                name.to_string(),
                value
                    .to_str()
                    .map_err(|_| "headers: invalid value")?
                    .to_owned(),
            ))
        })
        .collect::<Result<_, String>>()?;
    Ok(())
}
