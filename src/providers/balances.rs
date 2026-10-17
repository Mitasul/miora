//! Read-only upstream billing queries, isolated from routing and model metrics.

use crate::config::snapshot::Provider;
use futures_util::{stream, FutureExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

mod accounts;
mod store;
pub(crate) use accounts::{query_channel, AccountBalance};
pub(crate) use store::BalanceStore;

const MAX_KEYS: usize = 8;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn failure(status: &str) -> Value {
    json!({"status":status})
}
