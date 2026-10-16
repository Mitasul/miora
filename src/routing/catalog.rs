//! Read-only projections of configured channel order. This module never invokes

//! route scheduling, key selection, rate admission, or upstream requests.

use crate::config::snapshot::ApiKey;
use crate::config::snapshot::Provider;
use crate::config::snapshot::Snapshot;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;

type Entry = (Arc<Provider>, String);

pub(crate) fn key_id(token: &str) -> String {
    format!("key-{:x}", Sha256::digest(token.as_bytes()))
}

// Cross-key catalog visibility requires an explicit administrator role.
pub(crate) fn can_inspect_all(_snapshot: &Snapshot, caller: &ApiKey) -> bool {
    caller.role.eq_ignore_ascii_case("admin")
}

pub(crate) fn keys(snapshot: &Snapshot, caller: &ApiKey) -> Vec<Value> {
    snapshot.api_key_order.iter().enumerate()
        .filter_map(|(index, token)| {
            snapshot.api_keys.get(token)?;
            // Never return a complete credential, including unusually short keys.
            let prefix = if token.chars().count() > 14 {
                format!("{}…{}", token.chars().take(7).collect::<String>(), token.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>())
            } else { "••••".to_owned() };
            Some(json!({"key_id":key_id(token),"prefix":prefix,"position":index+1,"is_current":token==caller.token.as_ref()}))
        }).collect()
}

pub(crate) fn entries(
    snapshot: &Snapshot,
    caller: &ApiKey,
    selected_id: Option<&str>,
) -> Result<Vec<Entry>, u16> {
    if !can_inspect_all(snapshot, caller) {
        return Err(403);
    }
    if let Some(id) = selected_id.filter(|id| !id.is_empty()) {
        let selected = snapshot
            .api_key_order
            .iter()
            .filter_map(|token| snapshot.api_keys.get(token))
            .find(|key| key_id(&key.token) == id)
            .ok_or(404u16)?;
        return Ok(ordered_entries(snapshot, selected)
            .into_iter()
            .filter(|(p, _)| crate::control::channels::temporary_allowed(p, selected))
            .collect());
    }
    Ok(snapshot
        .providers
        .iter()
        .flat_map(|p| provider_entries(p, None))
        .collect())
}

fn provider_entries(provider: &Arc<Provider>, model: Option<&str>) -> Vec<Entry> {
    if let Some(model) = model {
        return if provider.models.contains_key(model) {
            vec![(provider.clone(), model.to_owned())]
        } else {
            Vec::new()
        };
    }
    let models: BTreeSet<_> = provider.models.keys().cloned().collect();
    models
        .into_iter()
        .map(|name| (provider.clone(), name))
        .collect()
}

#[derive(Default)]
struct Expansion {
    entries: Vec<Entry>,
    seen_entries: BTreeSet<(String, String, String)>,
    expanded_keys: BTreeSet<(String, Option<String>, Vec<String>)>,
}

impl Expansion {
    fn append(&mut self, entries: Vec<Entry>, scope: &[&str]) {
        for (provider, model) in entries {
            if !matches_groups(&provider.preferences, scope) {
                continue;
            }
            if self.seen_entries.insert((
                provider.name.to_string(),
                model.clone(),
                provider.models.get(&model).cloned().unwrap_or_default(),
            )) {
                self.entries.push((provider, model));
            }
        }
    }
    fn walk(
        &mut self,
        snapshot: &Snapshot,
        key: &ApiKey,
        only_model: Option<&str>,
        inherited: Option<&[&str]>,
    ) {
        let mut scope: Vec<_> = groups(&key.preferences)
            .into_iter()
            .filter(|group| inherited.is_none_or(|parent| parent.contains(group)))
            .collect();
        if scope.is_empty() {
            return;
        }
        scope.sort_unstable();
        // Memoizing each key/model pair preserves first occurrence order, avoids
        // exponential expansion of shared subgraphs, and terminates cycles.
        if !self.expanded_keys.insert((
            key.token.to_string(),
            only_model.map(str::to_owned),
            scope.iter().map(|group| (*group).to_owned()).collect(),
        )) {
            return;
        }
        let rules: Vec<&str> = key
            .preferences
            .get("__route_graph")
            .and_then(Value::as_array)
            .map(|rules| rules.iter().filter_map(Value::as_str).collect())
            .unwrap_or_else(|| key.model_rules.iter().map(String::as_str).collect());
        for rule in rules {
            let rule = rule.trim();
            if rule == "all" {
                for provider in snapshot.providers.iter() {
                    self.append(provider_entries(provider, only_model), &scope);
                }
                let mut aliases = snapshot.virtual_models.keys().collect::<Vec<_>>();
                aliases.sort();
                for alias in aliases {
                    self.global_model(snapshot, &scope, alias, only_model);
                }
            } else if let Some(model) = rule.strip_prefix('<').and_then(|v| v.strip_suffix('>')) {
                self.global_model(snapshot, &scope, model, only_model);
            } else if let Some((alias, requested)) = rule.split_once('/') {
                if requested != "*" && only_model.is_some_and(|wanted| wanted != requested) {
                    continue;
                }
                let model = if requested == "*" {
                    only_model
                } else {
                    Some(requested)
                };
                if let Some(child) = snapshot.api_keys.get(alias) {
                    self.walk(snapshot, child, model, Some(&scope));
                } else if let Some(provider) = snapshot.providers_by_name.get(alias) {
                    self.append(provider_entries(provider, model), &scope);
                }
            } else {
                self.global_model(snapshot, &scope, rule, only_model);
            }
        }
    }
    fn global_model(
        &mut self,
        snapshot: &Snapshot,
        scope: &[&str],
        model: &str,
        only_model: Option<&str>,
    ) {
        if only_model.is_some_and(|wanted| wanted != model) {
            return;
        }
        if let Some(routes) = snapshot.virtual_models.get(model) {
            self.append(
                routes
                    .iter()
                    .map(|provider| (provider.clone(), model.to_owned()))
                    .collect(),
                scope,
            );
            return;
        }
        for provider in snapshot.providers.iter() {
            self.append(provider_entries(provider, Some(model)), scope);
        }
    }
}

pub(crate) fn ordered_entries(snapshot: &Snapshot, key: &ApiKey) -> Vec<Entry> {
    let mut expansion = Expansion::default();
    expansion.walk(snapshot, key, None, None);
    expansion.entries
}

pub(crate) fn model_entries(snapshot: &Snapshot, key: &ApiKey, model: &str) -> Vec<Entry> {
    let mut expansion = Expansion::default();
    expansion.walk(snapshot, key, Some(model), None);
    expansion.entries
}

pub(crate) fn groups(preferences: &serde_json::Map<String, Value>) -> Vec<&str> {
    let values: Vec<_> = preferences
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if values.is_empty() {
        vec!["default"]
    } else {
        values
    }
}

pub(crate) fn group_allowed(provider: &Provider, key: &ApiKey) -> bool {
    matches_groups(&provider.preferences, &groups(&key.preferences))
}

fn matches_groups(preferences: &serde_json::Map<String, Value>, scope: &[&str]) -> bool {
    match preferences.get("groups").and_then(Value::as_array) {
        Some(groups) => groups
            .iter()
            .any(|group| group.as_str().is_some_and(|group| scope.contains(&group))),
        None => scope.contains(&"default"),
    }
}

#[cfg(test)]
mod workspace_features {
    use super::*;

    #[test]
    fn group_membership_is_an_intersection_not_a_label() {
        let preferences = serde_json::from_value(json!({"groups":["prod","shared"]})).unwrap();
        assert!(matches_groups(&preferences, &["prod"]));
        assert!(matches_groups(&preferences, &["dev", "shared"]));
        assert!(!matches_groups(&preferences, &["dev"]));
        assert!(!matches_groups(&preferences, &[]));
        assert!(matches_groups(&serde_json::Map::new(), &["default"]));
        assert!(!matches_groups(&serde_json::Map::new(), &["prod"]));
    }

    #[test]
    fn disabled_keys_are_not_compiled() {
        let mut row =
            json!({"api":"fixture","name":"development","groups":["dev"],"enabled":false});
        assert!(crate::config::compiler::compile_api_key(&row, true).is_none());
        row["enabled"] = json!(true);
        let compiled = crate::config::compiler::compile_api_key(&row, true).unwrap();
        assert_eq!(compiled["preferences"]["groups"], json!(["dev"]));
        assert_eq!(compiled["preferences"]["name"], "development");
    }
}
