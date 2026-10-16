use crate::config::snapshot::Provider;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

pub(crate) fn compile(config: &Value) -> Result<Value, String> {
    let Some(definitions) = config.get("virtual_models") else {
        return Ok(json!({}));
    };
    let definitions = definitions
        .as_object()
        .ok_or("virtual_models must be a mapping of names to target arrays")?;
    if definitions.len() > 256 {
        return Err("At most 256 virtual models are supported".into());
    }
    let providers: HashMap<_, _> = config["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let compiled = crate::config::compiler::compile_provider(row)?;
            Some((compiled["name"].as_str()?.to_owned(), compiled))
        })
        .collect();
    let mut completed = BTreeMap::new();
    for name in definitions.keys() {
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err("Virtual model names must contain 1-256 non-control bytes".into());
        }
        if providers
            .values()
            .any(|provider| provider["models"].get(name).is_some())
        {
            return Err(format!(
                "Virtual model conflicts with a channel model: {name}"
            ));
        }
        expand(
            name,
            definitions,
            &providers,
            &mut BTreeSet::new(),
            &mut completed,
        )?;
    }
    Ok(json!(completed))
}

fn expand(
    name: &str,
    definitions: &Map<String, Value>,
    providers: &HashMap<String, Value>,
    active: &mut BTreeSet<String>,
    completed: &mut BTreeMap<String, Vec<Value>>,
) -> Result<Vec<Value>, String> {
    if let Some(targets) = completed.get(name) {
        return Ok(targets.clone());
    }
    if !active.insert(name.to_owned()) {
        return Err(format!(
            "Virtual model cycle: {} -> {name}",
            active.iter().cloned().collect::<Vec<_>>().join(" -> ")
        ));
    }
    let targets = definitions
        .get(name)
        .and_then(Value::as_array)
        .filter(|targets| !targets.is_empty())
        .ok_or_else(|| format!("Virtual model {name} requires a nonempty target array"))?;
    let mut expanded = Vec::new();
    for target in targets {
        if let Some(child) = target.get("virtual").and_then(Value::as_str) {
            if target.get("on_status").is_some() {
                return Err("on_status belongs on concrete virtual-model targets".into());
            }
            expanded.extend(expand(child, definitions, providers, active, completed)?);
        } else {
            let provider_name = target["provider"]
                .as_str()
                .ok_or("Virtual target provider is required")?;
            let model = target["model"]
                .as_str()
                .ok_or("Virtual target model is required")?;
            let provider = providers
                .get(provider_name)
                .ok_or_else(|| format!("Unknown virtual-model provider: {provider_name}"))?;
            if provider["models"].get(model).is_none() {
                return Err(format!("Unknown virtual target: {provider_name}/{model}"));
            }
            if let Some(statuses) = target.get("on_status") {
                if statuses.as_array().is_none_or(|statuses| {
                    statuses.is_empty()
                        || statuses.iter().any(|status| {
                            !status
                                .as_u64()
                                .is_some_and(|status| (400..=599).contains(&status))
                        })
                }) {
                    return Err(
                        "Virtual target on_status must be an array of HTTP error statuses".into(),
                    );
                }
            }
            expanded.push(target.clone());
        }
        if expanded.len() > 4096 {
            return Err(format!("Virtual model {name} expands beyond 4096 targets"));
        }
    }
    if expanded
        .first()
        .is_some_and(|target| target.get("on_status").is_some())
    {
        return Err(format!(
            "First target of virtual model {name} must be unconditional"
        ));
    }
    active.remove(name);
    completed.insert(name.to_owned(), expanded.clone());
    Ok(expanded)
}

pub(crate) fn runtime(
    preferences: &Map<String, Value>,
    providers: &HashMap<String, Arc<Provider>>,
) -> HashMap<String, Vec<Arc<Provider>>> {
    preferences
        .get("__virtual_models")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(alias, targets)| {
            let routes = targets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|target| {
                    let provider = providers.get(target["provider"].as_str()?)?;
                    let model = target["model"].as_str()?;
                    let upstream = provider.models.get(model)?;
                    let mut route = (**provider).clone();
                    Arc::make_mut(&mut route.models).insert(alias.clone(), upstream.clone());
                    let preferences = Arc::make_mut(&mut route.preferences);
                    preferences.insert("__virtual_alias".into(), json!(alias));
                    preferences.insert("__virtual_target".into(), json!(model));
                    if let Some(pricing) = provider
                        .preferences
                        .get("usage_pricing")
                        .and_then(|prices| prices.get("models"))
                        .and_then(|models| models.get(model))
                        .cloned()
                    {
                        let prices = preferences
                            .entry("usage_pricing")
                            .or_insert_with(|| json!({}));
                        if prices.get("models").is_none() {
                            prices["models"] = json!({});
                        }
                        prices["models"][alias] = pricing;
                    }
                    if let Some(statuses) = target.get("on_status") {
                        preferences.insert("__virtual_on_status".into(), statuses.clone());
                    }
                    Some(Arc::new(route))
                })
                .collect();
            (alias.clone(), routes)
        })
        .collect()
}
