//! Settings for the independent request-log store, not the statistics database.
use super::CaptureField;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LogSettings {
    pub enabled: bool,
    pub capture_fields: Vec<CaptureField>,
    pub body_retention_hours: u32,
    pub row_retention: RowRetention,
    pub limits: LogLimits,
    pub runtime: RuntimeRotation,
}
impl Default for LogSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_fields: Vec::new(),
            body_retention_hours: 0,
            row_retention: RowRetention::default(),
            limits: LogLimits::default(),
            runtime: RuntimeRotation::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RowRetention {
    pub mode: RetentionMode,
    pub days: u32,
}
impl Default for RowRetention {
    fn default() -> Self {
        Self {
            mode: RetentionMode::Manual,
            days: 30,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetentionMode {
    Manual,
    Days,
    Forever,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LogLimits {
    pub per_request_bytes: u64,
    pub total_body_bytes: u64,
}
impl Default for LogLimits {
    fn default() -> Self {
        Self {
            per_request_bytes: 1_048_576,
            total_body_bytes: 268_435_456,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RuntimeRotation {
    pub file_max_bytes: u64,
    pub retained_files: u32,
}
impl Default for RuntimeRotation {
    fn default() -> Self {
        Self {
            file_max_bytes: 10_485_760,
            retained_files: 3,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct SettingsSnapshot {
    pub revision: u64,
    pub settings: LogSettings,
    pub backend_enabled: bool,
    pub runtime_rotation_managed: bool,
}
impl SettingsSnapshot {
    pub fn body_expires_at_ms(&self, at_ms: i64) -> Option<i64> {
        (self.backend_enabled && self.settings.enabled && self.settings.body_retention_hours > 0)
            .then(|| {
                at_ms.saturating_add(i64::from(self.settings.body_retention_hours) * 3_600_000)
            })
    }
}
impl LogSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.body_retention_hours > 87_600 {
            return Err("body_retention_hours must be 0-87600".into());
        }
        if self.row_retention.mode == RetentionMode::Days
            && !(1..=36_500).contains(&self.row_retention.days)
        {
            return Err("row_retention.days must be 1-36500 in days mode".into());
        }
        if !(1..=16_777_216).contains(&self.limits.per_request_bytes) {
            return Err("limits.per_request_bytes must be 1-16777216".into());
        }
        if self.limits.total_body_bytes > 1_099_511_627_776 {
            return Err("limits.total_body_bytes must not exceed 1 TiB".into());
        }
        if !(1024..=1_073_741_824).contains(&self.runtime.file_max_bytes)
            || !(1..=32).contains(&self.runtime.retained_files)
        {
            return Err(
                "runtime requires file_max_bytes 1024-1073741824 and retained_files 1-32".into(),
            );
        }
        let mut fields = self.capture_fields.clone();
        fields.sort();
        fields.dedup();
        if fields.len() != self.capture_fields.len() {
            return Err("capture_fields must not contain duplicates".into());
        }
        Ok(())
    }
}
