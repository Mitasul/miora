//! Retention and two-phase cleanup affect only this module's tables/body files.
use super::config::{LogSettings, RetentionMode};
use super::store::{CleanupAction, CleanupRequest, Database, Failure};
use super::CaptureField;
use axum::http::StatusCode;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde_json::{json, Value};

impl Database {
    pub(super) fn clear_one(
        &self,
        id: &str,
        attempt: i64,
        field: &str,
        availability: &str,
        reason: &str,
    ) -> Result<(bool, u64), Failure> {
        let current: Option<(Option<String>,String)> = self.conn.query_row("SELECT file_path,availability FROM request_log_fields WHERE record_id=? AND attempt=? AND field=?", params![id,attempt,field], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        let Some((path, previous)) = current else {
            return Ok((false, 0));
        };
        let mut bytes = 0;
        if let Some(path) = path {
            let path = self.directory.join(path);
            bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    bytes = 0;
                }
                Err(_) => return Err(Failure::unavailable()),
            }
        }
        self.conn.execute("UPDATE request_log_fields SET availability=?,availability_reason=?,file_path=NULL WHERE record_id=? AND attempt=? AND field=?", params![availability,reason,id,attempt,field])?;
        Ok((previous != availability || bytes > 0, bytes))
    }
    pub(super) fn evict_to_fit(&self, limit: u64, incoming: u64) -> Result<(), Failure> {
        let mut total: u64 = self.conn.query_row("SELECT COALESCE(SUM(saved_bytes),0) FROM request_log_fields WHERE availability='available'", [], |r|r.get(0))?;
        while total.saturating_add(incoming) > limit {
            let oldest: Option<(String,i64,String,u64)> = self.conn.query_row("SELECT f.record_id,f.attempt,f.field,f.saved_bytes FROM request_log_fields f JOIN request_log_rows r ON r.id=f.record_id WHERE f.availability='available' ORDER BY r.at_ms,f.record_id,f.attempt,f.field LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            let Some((id, attempt, field, size)) = oldest else {
                break;
            };
            self.clear_one(&id, attempt, &field, "evicted", "total_body_limit")?;
            total = total.saturating_sub(size);
        }
        Ok(())
    }
    pub(super) fn gc(&self, settings: &LogSettings, now: i64) -> Result<(), Failure> {
        loop {
            let rows: Vec<(String,i64,String)> = self.conn.prepare("SELECT record_id,attempt,field FROM request_log_fields WHERE availability='available' AND expires_at_ms IS NOT NULL AND expires_at_ms<=? LIMIT 256")?
                .query_map([now], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<Result<_,_>>()?;
            if rows.is_empty() {
                break;
            }
            for (id, attempt, field) in rows {
                self.clear_one(&id, attempt, &field, "expired", "retention_expired")?;
            }
        }
        self.evict_to_fit(settings.limits.total_body_bytes, 0)?;
        if settings.row_retention.mode == RetentionMode::Days {
            let cutoff = now.saturating_sub(i64::from(settings.row_retention.days) * 86_400_000);
            let mut after = String::new();
            loop {
                let ids: Vec<String> = self.conn.prepare("SELECT id FROM request_log_rows WHERE active=0 AND at_ms<? AND id>? ORDER BY id LIMIT 256")?
                    .query_map(params![cutoff,after], |r|r.get(0))?.collect::<Result<_,_>>()?;
                if ids.is_empty() {
                    break;
                }
                for id in &ids {
                    let _ = self.delete_row(id)?;
                }
                after = ids.last().unwrap().clone();
            }
        }
        self.conn
            .execute("DELETE FROM cleanup_plans WHERE expires_at_ms<=?", [now])?;
        Ok(())
    }
    fn fields_for(
        &self,
        id: &str,
        selected: &[CaptureField],
    ) -> Result<Vec<(i64, String)>, Failure> {
        let all = self
            .conn
            .prepare("SELECT attempt,field FROM request_log_fields WHERE record_id=?")?
            .query_map([id], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(all
            .into_iter()
            .filter(|(_, field)| {
                selected.is_empty() || selected.iter().any(|selected| selected.as_str() == field)
            })
            .collect())
    }
    fn delete_row(&self, id: &str) -> Result<(bool, u64, u64), Failure> {
        let active: Option<bool> = self
            .conn
            .query_row(
                "SELECT active FROM request_log_rows WHERE id=?",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if active != Some(false) {
            return Ok((false, 0, 0));
        }
        let mut freed = 0;
        let mut fields = 0;
        let mut failed = false;
        for (attempt, field) in self.fields_for(id, &[])? {
            match self.clear_one(id, attempt, &field, "cleared", "row_deleted") {
                Ok((changed, bytes)) => {
                    freed += bytes;
                    if changed {
                        fields += 1;
                    }
                }
                Err(_) => failed = true,
            }
        }
        let deleted = !failed
            && self
                .conn
                .execute("DELETE FROM request_log_rows WHERE id=? AND active=0", [id])
                .unwrap_or(0)
                > 0;
        Ok((deleted, freed, fields))
    }
    pub(super) fn cleanup(&self, input: CleanupRequest, now: i64) -> Result<Value, Failure> {
        if input.dry_run {
            return self.plan_cleanup(input, now);
        }
        let token = input
            .confirmation_token
            .ok_or_else(|| Failure::bad("confirmation_token from a cleanup dry run is required"))?;
        let plan: Option<(i64, String, String)> = self
            .conn
            .query_row(
                "SELECT expires_at_ms,action,fields_json FROM cleanup_plans WHERE token=?",
                [&token],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (expires, action, fields) = plan.ok_or_else(|| {
            Failure::new(
                StatusCode::CONFLICT,
                "cleanup_confirmation_expired",
                "Cleanup confirmation is missing, expired or already used",
            )
        })?;
        if expires <= now {
            self.conn
                .execute("DELETE FROM cleanup_plans WHERE token=?", [&token])?;
            return Err(Failure::new(
                StatusCode::CONFLICT,
                "cleanup_confirmation_expired",
                "Cleanup preview expired; preview the target set again",
            ));
        }
        let fields: Vec<CaptureField> =
            serde_json::from_str(&fields).map_err(|_| Failure::unavailable())?;
        let mut affected = 0u64;
        let mut cleared = 0u64;
        let mut freed = 0u64;
        let mut skipped = 0u64;
        let mut after = String::new();
        loop {
            let ids: Vec<String> = self.conn.prepare("SELECT record_id FROM cleanup_targets WHERE token=? AND record_id>? ORDER BY record_id LIMIT 256")?
                .query_map(params![token,after],|r|r.get(0))?.collect::<Result<_,_>>()?;
            if ids.is_empty() {
                break;
            }
            for id in &ids {
                let active: Option<bool> = self
                    .conn
                    .query_row(
                        "SELECT active FROM request_log_rows WHERE id=?",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if active != Some(false) {
                    skipped += 1;
                    continue;
                }
                if action == "delete_rows" {
                    match self.delete_row(id) {
                        Ok((deleted, bytes, count)) => {
                            if deleted {
                                affected += 1;
                            } else {
                                skipped += 1;
                            }
                            freed += bytes;
                            cleared += count;
                        }
                        Err(_) => skipped += 1,
                    }
                } else {
                    let mut changed = false;
                    let mut failed = false;
                    for (attempt, field) in self.fields_for(id, &fields)? {
                        match self.clear_one(id, attempt, &field, "cleared", "manual_cleanup") {
                            Ok((did_change, bytes)) => {
                                if did_change {
                                    cleared += 1;
                                    changed = true;
                                }
                                freed += bytes;
                            }
                            Err(_) => failed = true,
                        }
                    }
                    if changed {
                        affected += 1;
                    }
                    if failed {
                        skipped += 1;
                    }
                }
            }
            after = ids.last().unwrap().clone();
        }
        self.conn
            .execute("DELETE FROM cleanup_plans WHERE token=?", [&token])?;
        Ok(
            json!({"dry_run":false,"action":action,"affected_rows":affected,"cleared_fields":cleared,"freed_bytes":freed,"skipped_rows":skipped}),
        )
    }
    fn plan_cleanup(&self, input: CleanupRequest, now: i64) -> Result<Value, Failure> {
        let action = input
            .action
            .ok_or_else(|| Failure::bad("cleanup action is required"))?;
        if matches!(action, CleanupAction::ClearFields) && input.fields.is_empty() {
            return Err(Failure::bad(
                "clear_fields requires at least one selected capture field",
            ));
        }
        if input
            .filters
            .ids
            .as_ref()
            .is_some_and(|ids| ids.len() > 1000)
        {
            return Err(Failure::bad("At most 1000 selected ids may be supplied"));
        }
        self.conn
            .execute("DELETE FROM cleanup_plans WHERE expires_at_ms<=?", [now])?;
        let plans: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM cleanup_plans", [], |r| r.get(0))?;
        if plans >= 32 {
            return Err(Failure::new(
                StatusCode::CONFLICT,
                "too_many_cleanup_previews",
                "Too many active cleanup previews; confirm one or wait for expiry",
            ));
        }
        let (clause, values) = input.filters.sql();
        let count: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM request_log_rows r WHERE ({clause}) AND r.active=0"),
            params_from_iter(values.iter()),
            |r| r.get(0),
        )?;
        let active: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM request_log_rows r WHERE ({clause}) AND r.active=1"),
            params_from_iter(values.iter()),
            |r| r.get(0),
        )?;
        let token = super::random_token().map_err(Failure::bad)?;
        let expires = now.saturating_add(300_000);
        let action = match action {
            CleanupAction::ClearFields => "clear_fields",
            CleanupAction::DeleteRows => "delete_rows",
        };
        self.conn.execute("INSERT INTO cleanup_plans(token,expires_at_ms,action,fields_json,filters_json,matched_rows,active_excluded,estimated_bytes) VALUES(?,?,?,?,?,?,?,0)",params![token,expires,action,serde_json::to_string(&input.fields).unwrap(),serde_json::to_string(&input.filters).unwrap(),count,active])?;
        let mut target_values = vec![rusqlite::types::Value::from(token.clone())];
        target_values.extend(values);
        self.conn.execute(&format!("INSERT INTO cleanup_targets(token,record_id) SELECT ?,r.id FROM request_log_rows r WHERE ({clause}) AND r.active=0"),params_from_iter(target_values.iter()))?;
        let mut estimate_values = vec![rusqlite::types::Value::from(token.clone())];
        let selected = if action == "clear_fields" {
            for field in &input.fields {
                estimate_values.push(field.as_str().to_owned().into());
            }
            format!(
                " AND f.field IN ({})",
                vec!["?"; input.fields.len()].join(",")
            )
        } else {
            String::new()
        };
        let estimated:i64=self.conn.query_row(&format!("SELECT COALESCE(SUM(f.saved_bytes),0) FROM request_log_fields f JOIN cleanup_targets t ON t.record_id=f.record_id WHERE t.token=? AND f.availability='available'{selected}"),params_from_iter(estimate_values.iter()),|r|r.get(0))?;
        self.conn.execute(
            "UPDATE cleanup_plans SET estimated_bytes=? WHERE token=?",
            params![estimated, token],
        )?;
        Ok(
            json!({"dry_run":true,"confirmation_token":token,"expires_at_ms":expires,"matched_rows":count,"active_excluded":active,"estimated_bytes":estimated,"action":action,"fields":input.fields,"filters":input.filters}),
        )
    }
}
