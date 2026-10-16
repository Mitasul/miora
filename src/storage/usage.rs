use super::database::RequestStat;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio_postgres::Client;

const PRICE_FIELDS: [&str; 5] = [
    "input",
    "output",
    "cache_read",
    "cache_write",
    "cache_write_1h",
];

fn valid_rate(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|price| price.is_finite() && *price >= 0.0 && *price <= 1_000_000.0)
}

pub(crate) fn pricing_for_model(prices: Option<&Value>, model: &str) -> Option<Value> {
    let prices = prices?.as_object()?;
    let specific = prices
        .get("models")
        .and_then(|models| models.get(model))
        .and_then(Value::as_object);
    let mut selected = serde_json::Map::new();
    for key in PRICE_FIELDS {
        if let Some(value) = specific
            .and_then(|prices| prices.get(key))
            .or_else(|| prices.get(key))
        {
            selected.insert(
                key.into(),
                if valid_rate(value).is_some() {
                    value.clone()
                } else {
                    Value::Null
                },
            );
        }
    }
    Some(Value::Object(selected))
}

pub(crate) fn validate_pricing(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("usage_pricing must be an object")?;
    for (key, value) in object {
        if key == "models" {
            let models = value
                .as_object()
                .ok_or("pricing models must be an object")?;
            if models.len() > 500 {
                return Err("too many model prices".into());
            }
            for price in models.values() {
                validate_pricing(price)?;
            }
        } else if !PRICE_FIELDS.contains(&key.as_str())
            || (!value.is_null() && valid_rate(value).is_none())
        {
            return Err("prices must be finite non-negative USD amounts per million tokens".into());
        }
    }
    Ok(())
}

fn estimate(stat: &RequestStat) -> Option<f64> {
    let prices = stat.usage_pricing.as_ref()?;
    let rate = |key: &str| prices.get(key).and_then(valid_rate);
    let usage = &stat.fact_usage;
    let input = usage.input?;
    let output = usage.output?;
    let read = if input == 0 { 0 } else { usage.cache_read? };
    let write = usage.cache_write.unwrap_or(0);
    let hour = usage.cache_write_1h.unwrap_or(0);
    if hour > write || read.checked_add(write)? > input {
        return None;
    }
    let charge = |tokens: i64, key: &str| {
        if tokens == 0 {
            Some(0.0)
        } else {
            Some(tokens as f64 * rate(key)?)
        }
    };
    let cost = (charge(input - read - write, "input")?
        + charge(output, "output")?
        + charge(read, "cache_read")?
        + charge(write - hour, "cache_write")?
        + charge(hour, "cache_write_1h")?)
        / 1_000_000.0;
    cost.is_finite().then_some(cost)
}

pub(super) fn snapshot(stat: &RequestStat) -> String {
    let usage = &stat.fact_usage;
    let paired_input = usage
        .input
        .zip(usage.cache_read)
        .filter(|(input, read)| input >= read);
    json!({
        "input":usage.input,"output":usage.output,"cache_read":usage.cache_read,
        "total":usage.input.zip(usage.output).map(|(input,output)|input.saturating_add(output)),
        "cache_write":usage.cache_write,"cache_write_1h":usage.cache_write_1h,
        "cache_miss":paired_input.map(|(input, read)| input - read),
        "cache_input":paired_input.map(|(input, _)| input),
        "cache_hit":paired_input.map(|(_, read)| read),
        "status":(stat.status > 0).then_some(stat.status),
        "first_text_ms":stat.first_text_ms,"estimated_cost":estimate(stat),
        "pricing":stat.usage_pricing,"currency":"USD"
    })
    .to_string()
}

pub(super) fn migrate_sqlite(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(request_stats)")
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if !columns.iter().any(|column| column == "usage_details") {
        connection
            .execute_batch("ALTER TABLE request_stats ADD COLUMN usage_details TEXT;")
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn sql(postgres: bool, group: &str, bucket_seconds: i64) -> String {
    let field = |key: &str, real: bool| {
        let value = if postgres {
            format!("(usage_details::jsonb ->> '{key}')")
        } else {
            format!("json_extract(usage_details, '$.{key}')")
        };
        format!(
            "CAST({value} AS {})",
            if real { "DOUBLE PRECISION" } else { "BIGINT" }
        )
    };
    let bucket = if postgres {
        format!("CAST(FLOOR(EXTRACT(EPOCH FROM timestamp) / {bucket_seconds}) * {bucket_seconds} AS BIGINT)")
    } else {
        format!("(CAST(strftime('%s', timestamp) AS BIGINT) / {bucket_seconds}) * {bucket_seconds}")
    };
    let dimension = match group {
        "model" => "model".to_owned(),
        "channel" => "COALESCE(provider, '')".to_owned(),
        "trend" => format!("CAST({bucket} AS TEXT)"),
        _ => "''".to_owned(),
    };
    let mut columns = vec![
        dimension.clone(),
        "COUNT(*)".into(),
        "SUM(CASE WHEN NOT is_flagged THEN 1 ELSE 0 END)".into(),
    ];
    for key in [
        "input",
        "output",
        "cache_read",
        "cache_miss",
        "cache_write",
        "cache_write_1h",
        "estimated_cost",
    ] {
        let value = field(key, key == "estimated_cost");
        columns.push(if key == "estimated_cost" {
            format!("SUM({value})")
        } else {
            format!("CAST(SUM({value}) AS BIGINT)")
        });
        columns.push(format!("COUNT({value})"));
    }
    columns.extend([
        "AVG(process_time) * 1000.0".into(),
        format!("AVG({})", field("first_text_ms", true)),
        format!("CAST(SUM({}) AS BIGINT)", field("cache_input", false)),
        "COUNT(DISTINCT NULLIF(provider, ''))".into(),
        "CAST(SUM(prompt_tokens) AS BIGINT)".into(),
        "CAST(SUM(completion_tokens) AS BIGINT)".into(),
        format!("CAST(SUM({}) AS BIGINT)", field("cache_hit", false)),
        format!("CAST(SUM({}) AS BIGINT)", field("total", false)),
        format!("COUNT({})", field("total", false)),
    ]);
    columns.push(format!("'{group}' AS usage_group"));
    let suffix = match group {
        "model" | "channel" => {
            format!("GROUP BY {dimension} ORDER BY COUNT(*) DESC, {dimension} LIMIT 501")
        }
        "trend" => format!("GROUP BY {bucket} ORDER BY {bucket}"),
        _ => String::new(),
    };
    format!("SELECT {} FROM filtered_usage {suffix}", columns.join(", "))
}

struct UsageAggregate {
    dimension: String,
    group: String,
    metrics: Value,
}

fn row_value(
    counts: Vec<Option<i64>>,
    cost: Option<f64>,
    duration: Option<f64>,
    first_text: Option<f64>,
) -> Value {
    let count = counts[0].unwrap_or(0);
    let read = counts[6];
    let denominator = counts[15];
    json!({"request_count":count,"success_count":counts[1].unwrap_or(0),"failure_count":count.saturating_sub(counts[1].unwrap_or(0)),
        "input_tokens":counts[2],"input_samples":counts[3],"output_tokens":counts[4],"output_samples":counts[5],
        "cache_read_tokens":read,"cache_read_samples":counts[7],"cache_miss_tokens":counts[8],"cache_miss_samples":counts[9],
        "cache_write_tokens":counts[10],"cache_write_samples":counts[11],"cache_write_1h_tokens":counts[12],"cache_write_1h_samples":counts[13],
        "estimated_cost":cost,"priced_requests":counts[14],"currency":"USD","average_duration_ms":duration,"average_first_text_ms":first_text,
        "cache_hit_rate":counts[19].zip(denominator).and_then(|(read,input)| (input > 0).then_some(read as f64 / input as f64)),
        "channel_count":counts[16],"total_prompt_tokens":counts[17],"total_completion_tokens":counts[18],"api_key_prefix":"********",
        "total_tokens":counts[20],"total_samples":counts[21]})
}

fn sqlite_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageAggregate> {
    let indices = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 19, 20, 21, 22, 23, 24, 25,
    ];
    let counts = indices
        .iter()
        .map(|index| row.get(*index))
        .collect::<rusqlite::Result<Vec<Option<i64>>>>()?;
    Ok(UsageAggregate {
        dimension: row.get(0)?,
        group: row.get(26)?,
        metrics: row_value(counts, row.get(15)?, row.get(17)?, row.get(18)?),
    })
}

fn postgres_row(row: &tokio_postgres::Row) -> UsageAggregate {
    let indices = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 19, 20, 21, 22, 23, 24, 25,
    ];
    let counts = indices.iter().map(|index| row.get(*index)).collect();
    UsageAggregate {
        dimension: row.get(0),
        group: row.get(26),
        metrics: row_value(counts, row.get(15), row.get(17), row.get(18)),
    }
}

fn result(
    summary: Value,
    mut models: Vec<Value>,
    mut channels: Vec<Value>,
    trend: Vec<Value>,
    start: i64,
    end: i64,
) -> Value {
    let truncated = models.len() > 500;
    let channels_truncated = channels.len() > 500;
    models.truncate(500);
    channels.truncate(500);
    json!({"available":true,"summary":summary,"usage":models,"channels":channels,"trend":trend,"bucket_seconds":bucket_seconds(start,end),"truncated":truncated,"channels_truncated":channels_truncated,"from":start,"to":end,"currency":"USD","cost_kind":"configured_price_estimate","history_note":"reported_fields_only"})
}

fn bucket_seconds(start: i64, end: i64) -> i64 {
    if end - start <= 86_400 {
        3_600
    } else {
        86_400
    }
}

fn report_sql(postgres: bool, bucket_seconds: i64) -> String {
    let (start, end, model, channel) = if postgres {
        (
            "to_timestamp($1::bigint::double precision)",
            "to_timestamp($2::bigint::double precision)",
            "$3::text",
            "$4::text",
        )
    } else {
        (
            "datetime(?1, 'unixepoch')",
            "datetime(?2, 'unixepoch')",
            "?3",
            "?4",
        )
    };
    let groups = ["summary", "model", "channel", "trend"];
    let aggregates = groups
        .iter()
        .map(|group| {
            format!(
                "usage_{group} AS ({})",
                sql(postgres, group, bucket_seconds)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let selects = groups
        .iter()
        .map(|group| format!("SELECT * FROM usage_{group}"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    format!("WITH filtered_usage AS (SELECT model, provider, is_flagged, usage_details, process_time, prompt_tokens, completion_tokens, timestamp FROM request_stats WHERE model IS NOT NULL AND model != '' AND timestamp >= {start} AND timestamp < {end} AND ({model} IS NULL OR model = {model}) AND ({channel} IS NULL OR provider = {channel})), {aggregates} {selects}")
}

fn report(rows: Vec<UsageAggregate>, start: i64, end: i64) -> Result<Value, String> {
    let mut summary = None;
    let mut models = Vec::new();
    let mut channels = Vec::new();
    let mut trend = Vec::new();
    for row in rows {
        let mut metrics = row.metrics;
        match row.group.as_str() {
            "summary" => {
                metrics["model"] = row.dimension.into();
                summary = Some(metrics);
            }
            "model" => {
                metrics["model"] = row.dimension.into();
                models.push(metrics);
            }
            "channel" => {
                metrics["channel"] = row.dimension.into();
                channels.push(metrics);
            }
            "trend" => {
                let bucket = row
                    .dimension
                    .parse::<i64>()
                    .map_err(|error| error.to_string())?;
                metrics["bucket_start"] = bucket.into();
                trend.push(metrics);
            }
            _ => unreachable!("usage query emits four fixed groups"),
        }
    }
    trend.sort_by_key(|row| row["bucket_start"].as_i64());
    Ok(result(
        summary.ok_or("missing usage summary")?,
        models,
        channels,
        trend,
        start,
        end,
    ))
}

pub(super) fn sqlite(
    path: &PathBuf,
    start: i64,
    end: i64,
    model: Option<&str>,
    channel: Option<&str>,
) -> Result<Value, String> {
    let connection = super::database::sqlite_connection(path)?;
    let mut statement = connection
        .prepare(&report_sql(false, bucket_seconds(start, end)))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![start, end, model, channel], sqlite_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    report(rows, start, end)
}

pub(super) async fn postgres(
    client: &Client,
    start: i64,
    end: i64,
    model: Option<&str>,
    channel: Option<&str>,
) -> Result<Value, String> {
    let rows = client
        .query(
            &report_sql(true, bucket_seconds(start, end)),
            &[&start, &end, &model, &channel],
        )
        .await
        .map_err(|error| error.to_string())?;
    report(rows.iter().map(postgres_row).collect(), start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_report_keeps_one_scope_for_models_channels_and_trends() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE request_stats(model TEXT,provider TEXT,is_flagged INTEGER,process_time REAL,prompt_tokens INTEGER,completion_tokens INTEGER,timestamp TEXT,usage_details TEXT)").unwrap();
        let details = json!({"input":100,"output":10,"total":110,"cache_read":20,"cache_input":100,"cache_hit":20,"cache_miss":80,"estimated_cost":0.001});
        for (stamp, model, channel, failed, details) in [
            (3499, "m", "a", false, Some(details.clone())),
            (3500, "m", "a", false, Some(details.clone())),
            (4000, "m", "b", false, None),
            (5000, "m", "", false, None),
            (
                7200,
                "n",
                "a",
                true,
                Some(
                    json!({"input":0,"output":0,"total":0,"cache_read":0,"cache_input":0,"cache_hit":0,"cache_miss":0,"estimated_cost":0}),
                ),
            ),
            (7999, "n", "a", false, Some(details.clone())),
            (8000, "m", "a", false, Some(details)),
        ] {
            connection.execute("INSERT INTO request_stats VALUES(?1,?2,?3,1.0,100,10,datetime(?4,'unixepoch'),?5)", params![model, channel, failed, stamp, details.map(|value|value.to_string())]).unwrap();
        }
        let query = |model: Option<&str>, channel: Option<&str>, start: i64, end: i64| {
            let mut statement = connection
                .prepare(&report_sql(false, bucket_seconds(start, end)))
                .unwrap();
            let rows = statement
                .query_map(params![start, end, model, channel], sqlite_row)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            report(rows, start, end).unwrap()
        };
        let all = query(None, None, 3500, 8000);
        assert_eq!(all["summary"]["request_count"], 5);
        assert_eq!(all["summary"]["failure_count"], 1);
        assert_eq!(all["summary"]["input_samples"], 3);
        assert_eq!(all["summary"]["estimated_cost"], 0.002);
        assert_eq!(all["usage"].as_array().unwrap().len(), 2);
        assert_eq!(all["channels"][0]["channel"], "a");
        assert_eq!(all["channels"][0]["request_count"], 3);
        assert!(all["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["channel"] == "" && row["request_count"] == 1));
        assert_eq!(
            all["channels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["request_count"].as_i64().unwrap())
                .sum::<i64>(),
            5
        );
        assert!(all["channels"][1]["input_tokens"].is_null());
        assert_eq!(
            all["trend"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["bucket_start"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![0, 3600, 7200]
        );
        assert_eq!(all["trend"][2]["request_count"], 2);
        assert_eq!(all["trend"][2]["failure_count"], 1);
        let filtered = query(Some("m"), Some("a"), 3500, 8000);
        assert_eq!(filtered["summary"]["request_count"], 1);
        assert_eq!(filtered["channels"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["trend"].as_array().unwrap().len(), 1);
        let empty = query(None, Some("missing"), 3500, 8000);
        assert_eq!(empty["summary"]["request_count"], 0);
        assert!(empty["channels"].as_array().unwrap().is_empty());
        assert!(empty["trend"].as_array().unwrap().is_empty());
        let daily = query(None, None, 0, 7 * 86400);
        assert_eq!(daily["bucket_seconds"], 86400);
        assert_eq!(daily["trend"].as_array().unwrap().len(), 1);
        assert_eq!(daily["trend"][0]["request_count"], 7);
    }

    #[test]
    fn request_prices_only_copy_current_model_rates_and_preserve_unknowns() {
        let mut models = serde_json::Map::new();
        for index in 0..499 {
            models.insert(
                format!("other-{index}"),
                json!({"input":2,"output":8,"cache_read":0.2}),
            );
        }
        models.insert(
            "current".into(),
            json!({"input":0,"cache_read":null,"output":4}),
        );
        let catalog =
            json!({"input":2,"output":8,"cache_read":0.2,"cache_write":2.5,"models":models});
        let selected = pricing_for_model(Some(&catalog), "current").unwrap();
        assert_eq!(
            selected,
            json!({"input":0,"output":4,"cache_read":null,"cache_write":2.5})
        );
        assert!(selected.get("models").is_none());
        assert_eq!(
            pricing_for_model(Some(&json!({"input":-2,"output":"invalid"})), "current"),
            Some(json!({"input":null,"output":null}))
        );
        assert!(pricing_for_model(None, "current").is_none());
        println!(
            "500-model price catalog: {} bytes; current-model snapshot: {} bytes",
            catalog.to_string().len(),
            selected.to_string().len()
        );
    }

    #[test]
    fn prices_use_inclusive_input_without_double_charging_cache() {
        let mut stat = RequestStat {
            model: "example".into(),
            usage_pricing: Some(
                json!({"input":2,"output":8,"cache_read":0.2,"cache_write":2.5,"cache_write_1h":4}),
            ),
            ..Default::default()
        };
        stat.fact_usage = crate::observability::usage::FactUsage::from_usage(Some(
            &json!({"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":60}}),
        ));
        assert!((estimate(&stat).unwrap() - 0.000252).abs() < 1e-12);
        stat.fact_usage = crate::observability::usage::FactUsage::from_usage(Some(
            &json!({"input_tokens":5,"output_tokens":13,"cache_read_input_tokens":11,"cache_creation_input_tokens":7,"cache_creation":{"ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":5}}),
        ));
        assert!((estimate(&stat).unwrap() - 0.0001412).abs() < 1e-12);
        stat.usage_pricing = Some(json!({"input":2,"output":8}));
        assert!(estimate(&stat).is_none());
        stat.usage_pricing = None;
        assert!(estimate(&stat).is_none());
    }

    #[test]
    fn explicit_free_prices_model_overrides_and_unknown_usage_are_distinct() {
        let mut stat = RequestStat {
            model: "free".into(),
            usage_pricing: pricing_for_model(
                Some(&json!({"input":9,"output":9,"models":{"free":{"input":0,"output":0}}})),
                "free",
            ),
            ..Default::default()
        };
        stat.fact_usage.input = Some(10);
        stat.fact_usage.output = Some(20);
        stat.fact_usage.cache_read = Some(0);
        assert_eq!(estimate(&stat), Some(0.0));
        stat.fact_usage.cache_read = None;
        assert_eq!(estimate(&stat), None);
        stat.fact_usage.cache_read = Some(0);
        stat.fact_usage.output = None;
        assert_eq!(estimate(&stat), None);
        assert!(validate_pricing(&json!({"input":-1})).is_err());
        assert!(validate_pricing(&json!({"input":"2"})).is_err());
        assert!(validate_pricing(&json!({"unexpected":0})).is_err());
        assert!(validate_pricing(&json!({"input":null,"cache_read":null})).is_ok());
        assert!(validate_pricing(&json!({"unexpected":null})).is_err());
    }

    #[test]
    fn legacy_migration_preserves_rows_and_unknown_cache_is_not_zero() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE request_stats(model TEXT,provider TEXT,is_flagged INTEGER DEFAULT 0,process_time REAL,prompt_tokens INTEGER,completion_tokens INTEGER,timestamp TEXT); INSERT INTO request_stats VALUES('m','legacy',0,1.0,42,8,datetime(100,'unixepoch')); ").unwrap();
        migrate_sqlite(&connection).unwrap();
        migrate_sqlite(&connection).unwrap();
        let legacy = connection
            .query_row(
                &report_sql(false, 3600),
                params![0, 200, None::<String>, None::<String>],
                sqlite_row,
            )
            .unwrap();
        assert_eq!(legacy.metrics["request_count"], 1);
        assert!(legacy.metrics["input_tokens"].is_null());
        assert!(legacy.metrics["cache_read_tokens"].is_null());
        assert!(legacy.metrics["estimated_cost"].is_null());
        assert_eq!(legacy.metrics["total_prompt_tokens"], 42);
        let mut stat = RequestStat {
            model: "m".into(),
            ..Default::default()
        };
        stat.fact_usage.input = Some(100);
        stat.fact_usage.output = Some(20);
        stat.fact_usage.cache_read = Some(60);
        connection.execute("INSERT INTO request_stats VALUES('m','new',0,2.0,100,20,datetime(100,'unixepoch'),?1)", params![snapshot(&stat)]).unwrap();
        stat.fact_usage.input = None;
        stat.fact_usage.cache_read = Some(100);
        connection.execute("INSERT INTO request_stats VALUES('m','new',0,2.0,0,20,datetime(100,'unixepoch'),?1)", params![snapshot(&stat)]).unwrap();
        let result = connection
            .query_row(
                &report_sql(false, 3600),
                params![0, 200, None::<String>, None::<String>],
                sqlite_row,
            )
            .unwrap();
        assert_eq!(result.metrics["request_count"], 3);
        assert_eq!(result.metrics["input_samples"], 1);
        assert_eq!(result.metrics["cache_read_tokens"], 160);
        assert_eq!(result.metrics["cache_miss_tokens"], 40);
        assert_eq!(result.metrics["cache_hit_rate"], 0.6);
        let filtered = connection
            .query_row(
                &report_sql(false, 3600),
                params![0, 200, Some("m"), Some("legacy")],
                sqlite_row,
            )
            .unwrap();
        assert_eq!(filtered.metrics["request_count"], 1);
    }
}
