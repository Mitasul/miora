use super::manifest::Manifest;
use super::registry::{Registration, ServiceAuth};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::time::Duration;

const FILE_LIMIT: usize = 256 * 1024;
const BODY_LIMIT: usize = 17 * 1024 * 1024;
const FORMAT: &str = "miora.plugin.v1";
type UploadError = (StatusCode, String);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    format: String,
    manifest: Manifest,
    endpoint: String,
    #[serde(default)]
    config: Map<String, Value>,
    #[serde(default)]
    service_auth: ServiceAuth,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    expected_revision: u64,
}

pub(super) enum Upload {
    Service(Registration),
    Managed { bytes: bytes::Bytes, expected: u64 },
}

fn invalid(message: &str) -> UploadError {
    (StatusCode::BAD_REQUEST, message.to_owned())
}

fn decode(bytes: &[u8], options: Options) -> Result<Registration, UploadError> {
    if bytes.len() > FILE_LIMIT {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Plugin file exceeds 256 KiB".to_owned(),
        ));
    }
    let package: Package = serde_json::from_slice(bytes)
        .map_err(|_| invalid("Invalid Miora plugin file or unsupported fields"))?;
    if package.format != FORMAT {
        return Err(invalid("Expected a miora.plugin.v1 plugin file"));
    }
    Ok(Registration {
        manifest: package.manifest,
        endpoint: package.endpoint,
        config: package.config,
        service_auth: package.service_auth,
        expected_revision: options.expected_revision,
        managed: None,
        runtime: None,
    })
}

pub(super) async fn read(request: Request<Body>) -> Result<Upload, UploadError> {
    let boundary = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| multer::parse_boundary(value).ok())
        .ok_or_else(|| invalid("Expected multipart/form-data with a boundary"))?;
    let bytes = tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), BODY_LIMIT),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::REQUEST_TIMEOUT,
            "Plugin upload timed out".to_owned(),
        )
    })?
    .map_err(|_| {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "Plugin upload exceeds 17 MiB".to_owned(),
        )
    })?;
    let stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(bytes) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut file = None;
    let mut options = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| invalid("Malformed plugin upload"))?
    {
        match field.name() {
            Some("file") if file.is_none() => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| invalid("Unreadable plugin file"))?;
                file = Some(bytes);
            }
            Some("options") if options.is_none() && field.file_name().is_none() => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| invalid("Unreadable upload options"))?;
                options = Some(
                    serde_json::from_slice::<Options>(&bytes)
                        .map_err(|_| invalid("Invalid plugin upload options"))?,
                );
            }
            _ => return Err(invalid("Duplicate or unknown plugin upload field")),
        }
    }
    let file = file.ok_or_else(|| invalid("Select a plugin file"))?;
    let options = options.unwrap_or_default();
    if file.starts_with(b"PK\x03\x04") {
        if file.len() > 16 * 1024 * 1024 {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "Plugin archive exceeds 16 MiB".into(),
            ));
        }
        Ok(Upload::Managed {
            bytes: file,
            expected: options.expected_revision,
        })
    } else {
        decode(&file, options).map(Upload::Service)
    }
}

pub(super) fn template() -> Value {
    json!({
        "format":FORMAT,
        "manifest":{
            "id":"example-channel", "name":"Example channel", "version":"1.0.0", "api_version":1,
            "config_schema":[
                {"key":"base_url","label":"上游地址","type":"string","required":true},
                {"key":"api_key","label":"上游 API Key","type":"string","required":true,"secret":true}
            ],
            "auth":{"type":"bearer","config_key":"api_key"},
            "capabilities":{"endpoints":["/v1/chat/completions"],"streaming":true,"model_discovery":false},
            "models":[{"id":"example-model"}]
        },
        "endpoint":"https://plugin.example.com",
        "service_auth":{"type":"bearer","token":""},
        "config":{}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_package_preserves_falsy_config_and_revision() {
        let mut value = template();
        value["manifest"]["config_schema"] = json!([
            {"key":"enabled","type":"boolean"}, {"key":"limit","type":"integer"}
        ]);
        value["manifest"]["auth"] = json!({"type":"none"});
        value["config"] = json!({"enabled":false,"limit":0});
        let options = Options {
            expected_revision: 7,
        };
        let result = decode(&serde_json::to_vec(&value).unwrap(), options).unwrap();
        assert_eq!(result.expected_revision, 7);
        assert_eq!(result.config["enabled"], false);
        assert_eq!(result.config["limit"], 0);
    }

    #[test]
    fn rejects_foreign_formats_unknown_fields_and_invalid_metadata() {
        for field in ["format", "extra", "manifest"] {
            let mut value = template();
            value[field] = json!("unsupported");
            assert!(decode(&serde_json::to_vec(&value).unwrap(), Options::default()).is_err());
        }
        assert!(decode(b"import os", Options::default()).is_err());
        assert!(decode(&vec![b' '; FILE_LIMIT + 1], Options::default()).is_err());
    }

    #[test]
    fn package_size_limit_is_enforced_before_parsing() {
        let error = decode(&vec![b' '; FILE_LIMIT + 1], Options::default())
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(decode(b"", Options::default()).is_err());
    }

    #[test]
    fn upload_options_cannot_override_plugin_configuration() {
        assert!(serde_json::from_value::<Options>(json!({"config":{}})).is_err());
        assert!(
            serde_json::from_value::<Options>(json!({"service_auth":{"type":"none"}})).is_err()
        );
    }

    #[tokio::test]
    async fn multipart_accepts_one_native_file_and_rejects_duplicate_fields() {
        let payload = template().to_string();
        let file = format!("--miora-test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"local plugin.JSON\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n");
        for copies in [1, 2] {
            let body = file.repeat(copies) + "--miora-test--\r\n";
            let request = Request::builder()
                .header("content-type", "multipart/form-data; boundary=miora-test")
                .body(Body::from(body))
                .unwrap();
            assert_eq!(read(request).await.is_ok(), copies == 1);
        }
    }
}
