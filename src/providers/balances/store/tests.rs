use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "miora-balances-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn store(&self) -> BalanceStore {
        BalanceStore::at(self.0.join("balances.json"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn success(amount: f64) -> Value {
    json!({"provider":"channel","status":"complete","keys":[{"position":1,"status":"ok","amount":amount,"currency":"USD","checked_at":100}]})
}

#[tokio::test]
async fn balances_persistence_survives_restart_without_saving_credentials() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.read().await.unwrap().is_empty());
    assert!(!fixture.0.join("balances.json").exists());
    store
        .save("channel", "rev-1", success(3.5), &["channel".into()])
        .await
        .unwrap();
    let restarted = fixture.store().read().await.unwrap();
    assert_eq!(restarted["channel"]["keys"][0]["amount"], 3.5);
    assert_eq!(restarted["channel"]["config_revision"], "rev-1");
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
}

#[tokio::test]
async fn balances_failed_refresh_retains_success_then_success_clears_error() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let names = ["channel".into()];
    store
        .save("channel", "one", success(0.0), &names)
        .await
        .unwrap();
    let failure =
        json!({"status":"complete","keys":[{"position":1,"status":"timeout","checked_at":200}]});
    let saved = store.save("channel", "one", failure, &names).await.unwrap();
    assert_eq!(saved["keys"][0]["amount"], 0.0);
    assert_eq!(saved["keys"][0]["checked_at"], 100);
    assert_eq!(saved["keys"][0]["last_error"], "timeout");
    assert_eq!(saved["keys"][0]["last_attempt_at"], 200);
    let saved = store
        .save("channel", "one", success(7.0), &names)
        .await
        .unwrap();
    assert!(saved["keys"][0].get("last_error").is_none());
}

#[tokio::test]
async fn balances_new_config_or_account_does_not_inherit_another_account_amount() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let names = ["channel".into()];
    let mut initial = success(100.0);
    initial["keys"][0]["account_ref"] = json!("core-oauth:codex:old");
    store.save("channel", "one", initial, &names).await.unwrap();
    let failure = json!({"status":"complete","keys":[{"position":1,"account_ref":"core-oauth:codex:new","status":"auth_error","checked_at":200}]});
    let saved = store
        .save("channel", "one", failure.clone(), &names)
        .await
        .unwrap();
    assert!(saved["keys"][0].get("amount").is_none());
    store
        .save("channel", "one", success(100.0), &names)
        .await
        .unwrap();
    let saved = store.save("channel", "two", failure, &names).await.unwrap();
    assert!(saved["keys"][0].get("amount").is_none());
}

#[test]
fn balances_unsupported_or_disabled_does_not_resurrect_old_adapter_results() {
    for status in ["unsupported", "disabled", "no_key"] {
        let mut result = json!({"status":status,"keys":[],"checked_at":200});
        retain_success(&success(100.0), &mut result);
        assert_eq!(result["status"], status);
        assert_eq!(result["keys"], json!([]));
        assert!(result.get("last_error").is_none());
    }
}

#[tokio::test]
async fn balances_failed_persistence_does_not_publish_and_deleted_channels_are_pruned() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .save("deleted", "one", success(4.0), &["deleted".into()])
        .await
        .unwrap();
    store
        .save("channel", "one", success(3.0), &["channel".into()])
        .await
        .unwrap();
    assert!(!store.read().await.unwrap().contains_key("deleted"));
    let blocker = fixture.0.join("file");
    std::fs::write(&blocker, b"not-a-directory").unwrap();
    let failed = BalanceStore {
        path: Arc::new(blocker.join("balances.json")),
        values: store.values.clone(),
    };
    assert!(failed
        .save("channel", "one", success(50.0), &["channel".into()])
        .await
        .is_err());
    assert_eq!(
        store.read().await.unwrap()["channel"]["keys"][0]["amount"],
        3.0
    );
}
