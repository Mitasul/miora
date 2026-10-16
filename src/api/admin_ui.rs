//! Embedded, same-origin administrator UI. There is deliberately no filesystem
//! fallback: a request can only access the explicitly listed immutable assets.

use crate::runtime::context::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, Response, StatusCode};

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

pub(crate) fn handle(state: &AppState, request: &Request) -> Option<Response<Body>> {
    let path = request.uri().path();
    let normalized = path.trim_end_matches('/');
    let page = matches!(normalized, "" | "/login" | "/admin");
    let asset =
        path.starts_with("/admin/assets/") || matches!(path, "/favicon.svg" | "/favicon.ico");
    if !page && !asset {
        return None;
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let mut response = reply(
            StatusCode::METHOD_NOT_ALLOWED,
            "text/plain; charset=utf-8",
            "Method not allowed",
            false,
        );
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return Some(response);
    }
    let head = request.method() == Method::HEAD;
    if page {
        let authenticated = state.admin.is_authenticated(request.headers());
        if normalized.is_empty() {
            return Some(redirect(if authenticated { "/admin" } else { "/login" }));
        }
        if normalized == "/admin" && !authenticated {
            return Some(redirect("/login"));
        }
        if normalized == "/login" && authenticated {
            return Some(redirect("/admin"));
        }
        let html = if normalized == "/login" {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/login.html"
            ))
        } else {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/index.html"
            ))
        };
        return Some(reply(
            StatusCode::OK,
            "text/html; charset=utf-8",
            html,
            head,
        ));
    }
    let (mime, content) = match path {
        "/admin/assets/styles.css" => (
            "text/css; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/styles.css"
            )),
        ),
        "/admin/assets/app.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/admin/app.js")),
        ),
        "/admin/assets/theme.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/theme.js"
            )),
        ),
        "/admin/assets/login.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/login.js"
            )),
        ),
        "/admin/assets/api.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/admin/api.js")),
        ),
        "/admin/assets/ui.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/admin/ui.js")),
        ),
        "/admin/assets/request-logs.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/request-logs.js"
            )),
        ),
        "/admin/assets/log-settings.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/log-settings.js"
            )),
        ),
        "/admin/assets/workspace.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/workspace.js"
            )),
        ),
        "/admin/assets/backend-logs.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/backend-logs.js"
            )),
        ),
        "/admin/assets/channel-accounts.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/channel-accounts.js"
            )),
        ),
        "/admin/assets/playground.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/playground.js"
            )),
        ),
        "/admin/assets/virtual-models.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/virtual-models.js"
            )),
        ),
        "/admin/assets/model-testing.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/model-testing.js"
            )),
        ),
        "/admin/assets/key-manager.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/key-manager.js"
            )),
        ),
        "/admin/assets/channel-editor.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/channel-editor.js"
            )),
        ),
        "/admin/assets/parameters-editor.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/parameters-editor.js"
            )),
        ),
        "/admin/assets/models-editor.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/models-editor.js"
            )),
        ),
        "/admin/assets/icons.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/icons.js"
            )),
        ),
        "/admin/assets/plugin-manager.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/plugin-manager.js"
            )),
        ),
        "/admin/assets/usage-dashboard.js" => (
            "text/javascript; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/usage-dashboard.js"
            )),
        ),
        "/admin/assets/ai-icons/aws-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/aws-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/azure-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/azure-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/claude-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/claude-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/cloudflare-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cloudflare-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/cohere-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cohere-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/deepseek-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/deepseek-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/doubao-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/doubao-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/gemini-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/gemini-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/gemma-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/gemma-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/googlecloud-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/googlecloud-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/grok.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/grok.svg"
            )),
        ),
        "/admin/assets/ai-icons/huggingface-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/huggingface-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/hunyuan-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/hunyuan-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/kimi-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/kimi-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/meta-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/meta-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/minimax-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/minimax-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/mistral-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/mistral-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/nvidia-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/nvidia-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/ollama.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ollama.svg"
            )),
        ),
        "/admin/assets/ai-icons/openai.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/openai.svg"
            )),
        ),
        "/admin/assets/ai-icons/openrouter.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/openrouter.svg"
            )),
        ),
        "/admin/assets/ai-icons/perplexity-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/perplexity-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/qwen-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/qwen-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/siliconcloud-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/siliconcloud-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/stepfun-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/stepfun-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/volcengine-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/volcengine-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/wenxin-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/wenxin-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/yi-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/yi-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/zhipu-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/zhipu-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/ai2-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ai2-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/ai21.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ai21.svg"
            )),
        ),
        "/admin/assets/ai-icons/ai302-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ai302-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/aihubmix-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/aihubmix-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/alephalpha.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/alephalpha.svg"
            )),
        ),
        "/admin/assets/ai-icons/arcee-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/arcee-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/assemblyai-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/assemblyai-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/aya-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/aya-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/baai.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/baai.svg"
            )),
        ),
        "/admin/assets/ai-icons/baichuan-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/baichuan-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/baseten.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/baseten.svg"
            )),
        ),
        "/admin/assets/ai-icons/bfl.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/bfl.svg"
            )),
        ),
        "/admin/assets/ai-icons/briaai-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/briaai-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/cerebras-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cerebras-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/chutes.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/chutes.svg"
            )),
        ),
        "/admin/assets/ai-icons/cogvideo-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cogvideo-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/cogview-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cogview-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/cometapi-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/cometapi-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/coqui-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/coqui-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/dbrx-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/dbrx-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/deepcogito-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/deepcogito-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/deepinfra-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/deepinfra-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/elevenlabs.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/elevenlabs.svg"
            )),
        ),
        "/admin/assets/ai-icons/fal-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/fal-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/featherless-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/featherless-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/fireworks-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/fireworks-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/fishaudio.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/fishaudio.svg"
            )),
        ),
        "/admin/assets/ai-icons/flux.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/flux.svg"
            )),
        ),
        "/admin/assets/ai-icons/friendli.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/friendli.svg"
            )),
        ),
        "/admin/assets/ai-icons/giteeai.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/giteeai.svg"
            )),
        ),
        "/admin/assets/ai-icons/groq.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/groq.svg"
            )),
        ),
        "/admin/assets/ai-icons/hailuo-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/hailuo-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/huawei-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/huawei-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/hyperbolic-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/hyperbolic-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/ibm.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ibm.svg"
            )),
        ),
        "/admin/assets/ai-icons/ideogram.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ideogram.svg"
            )),
        ),
        "/admin/assets/ai-icons/inception.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/inception.svg"
            )),
        ),
        "/admin/assets/ai-icons/infinigence-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/infinigence-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/inflection.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/inflection.svg"
            )),
        ),
        "/admin/assets/ai-icons/internlm-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/internlm-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/jimeng-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/jimeng-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/jina.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/jina.svg"
            )),
        ),
        "/admin/assets/ai-icons/kling-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/kling-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/kolors-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/kolors-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/leptonai-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/leptonai-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/liquid.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/liquid.svg"
            )),
        ),
        "/admin/assets/ai-icons/lmstudio.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/lmstudio.svg"
            )),
        ),
        "/admin/assets/ai-icons/longcat-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/longcat-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/luma-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/luma-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/meshy-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/meshy-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/microsoft-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/microsoft-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/midjourney.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/midjourney.svg"
            )),
        ),
        "/admin/assets/ai-icons/modelscope-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/modelscope-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/nebius.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/nebius.svg"
            )),
        ),
        "/admin/assets/ai-icons/nousresearch.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/nousresearch.svg"
            )),
        ),
        "/admin/assets/ai-icons/nova-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/nova-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/novita-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/novita-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/openchat-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/openchat-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/pika.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/pika.svg"
            )),
        ),
        "/admin/assets/ai-icons/pixverse-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/pixverse-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/ppio-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/ppio-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/qiniu-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/qiniu-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/recraft.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/recraft.svg"
            )),
        ),
        "/admin/assets/ai-icons/reka.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/reka.svg"
            )),
        ),
        "/admin/assets/ai-icons/replicate.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/replicate.svg"
            )),
        ),
        "/admin/assets/ai-icons/runway.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/runway.svg"
            )),
        ),
        "/admin/assets/ai-icons/rwkv-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/rwkv-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/sambanova-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/sambanova-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/sensenova-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/sensenova-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/skywork-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/skywork-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/sophnet-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/sophnet-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/sora-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/sora-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/spark-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/spark-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/stability-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/stability-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/suno.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/suno.svg"
            )),
        ),
        "/admin/assets/ai-icons/tii-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/tii-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/together-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/together-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/tripo-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/tripo-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/udio-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/udio-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/upstage-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/upstage-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/vidu-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/vidu-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/vllm-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/vllm-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/voyage-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/voyage-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/xiaomimimo.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/xiaomimimo.svg"
            )),
        ),
        "/admin/assets/ai-icons/xinference-color.svg" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/xinference-color.svg"
            )),
        ),
        "/admin/assets/ai-icons/LICENSE" => (
            "text/plain; charset=utf-8",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/ai-icons/LICENSE"
            )),
        ),
        "/admin/assets/favicon.svg" | "/favicon.svg" | "/favicon.ico" => (
            "image/svg+xml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/admin/favicon.svg"
            )),
        ),
        _ => {
            return Some(reply(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                "Not found",
                head,
            ))
        }
    };
    Some(reply(StatusCode::OK, mime, content, head))
}

fn reply(
    status: StatusCode,
    mime: &'static str,
    content: &'static str,
    head: bool,
) -> Response<Body> {
    let mut response = Response::new(if head {
        Body::empty()
    } else {
        Body::from(content)
    });
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(content.len()));
    secure_headers(&mut response);
    response
}

fn redirect(location: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static(location));
    secure_headers(&mut response);
    response
}

fn secure_headers(response: &mut Response<Body>) {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("content-security-policy", CSP),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_has_no_body_but_preserves_representation_length() {
        let response = reply(StatusCode::OK, "text/plain", "hello", true);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "5");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
    }

    #[test]
    fn redirects_are_local_and_not_cacheable() {
        let response = redirect("/login");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/login");
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}
