"""
重命名遗留引用的回归测试。

7054078 把若干内部函数改成公开名后，有几处调用和旧导入名没有同步：
- 透传非流式慢路径（启用插件时）仍调用 _apply_response_path_interceptors，运行时报 NameError；
- core.passthrough 缺少 _fetch_passthrough_response 兼容别名，内置插件导入失败；
- OpenAI 图片渠道仍调用 _extract_prompt_and_images。
"""

import asyncio

import httpx

import core.passthrough as passthrough
from core.channels.openai_image_channel import get_image_payload
from core.models import RequestModel


def _run_passthrough_response(handler, **kwargs):
    transport = httpx.MockTransport(handler)

    async def run():
        async with httpx.AsyncClient(transport=transport) as client:
            return [
                chunk
                async for chunk in passthrough.fetch_passthrough_response(
                    client,
                    "https://upstream.example/v1/messages",
                    {"content-type": "application/json"},
                    {"model": "test-model"},
                    30,
                    engine="claude",
                    model="test-model",
                    **kwargs,
                )
            ]

    return asyncio.run(run())


def test_passthrough_non_stream_with_plugins_returns_upstream_body():
    body = '{"id": "msg_1", "content": [{"type": "text", "text": "PONG"}]}'
    chunks = _run_passthrough_response(
        lambda request: httpx.Response(200, text=body),
        enabled_plugins=["plugin_not_installed"],
    )
    assert chunks == [body]


def test_passthrough_non_stream_with_key_plugins_returns_upstream_error():
    chunks = _run_passthrough_response(
        lambda request: httpx.Response(429, json={"error": {"message": "rate limited"}}),
        key_enabled_plugins=["plugin_not_installed"],
    )
    assert len(chunks) == 1
    assert chunks[0]["status_code"] == 429
    assert chunks[0]["details"] == {"error": {"message": "rate limited"}}


def test_passthrough_keeps_private_fetch_aliases_for_plugins():
    # gemini_empty_retry、content_filter_retry 及第三方插件仍从 core.passthrough 导入旧名。
    from core.passthrough import _fetch_passthrough_response, _fetch_passthrough_stream

    assert _fetch_passthrough_response is passthrough.fetch_passthrough_response
    assert _fetch_passthrough_stream is passthrough.fetch_passthrough_stream


def test_openai_image_payload_extracts_prompt():
    request = RequestModel(
        model="gpt-image-2",
        messages=[{"role": "user", "content": "a red fox in the snow"}],
        stream=False,
    )
    provider = {"base_url": "https://api.example.com/v1", "model": ["gpt-image-2"]}

    url, headers, payload = asyncio.run(get_image_payload(request, "openai-image", provider, api_key="sk-test"))

    assert url == "https://api.example.com/v1/images/generations"
    assert headers["Authorization"] == "Bearer sk-test"
    assert payload["model"] == "gpt-image-2"
    assert payload["prompt"] == "a red fox in the snow"


def test_global_outbound_headers_and_default_user_agent():
    from core.http_headers import apply_global_outbound_headers
    from core.request import get_payload

    headers = {"Content-Type": "application/json"}
    apply_global_outbound_headers(headers, None, is_passthrough=False)
    assert headers["User-Agent"] == "Miora"

    headers_custom = {"Content-Type": "application/json"}
    apply_global_outbound_headers(
        headers_custom,
        {"user_agent": "CustomGateway/2.0", "headers": {"X-Title": "MyApp", "X-Custom-Tag": "v1"}},
        is_passthrough=False,
    )
    assert headers_custom["User-Agent"] == "CustomGateway/2.0"
    assert headers_custom["X-Title"] == "MyApp"
    assert headers_custom["X-Custom-Tag"] == "v1"

    # get_payload integrates global_preferences and channel headers override
    req = RequestModel(
        model="gpt-4o",
        messages=[{"role": "user", "content": "hi"}],
        stream=False,
    )
    provider = {
        "provider": "openai-test",
        "base_url": "https://api.example.com/v1",
        "model": ["gpt-4o"],
        "preferences": {"headers": {"X-Channel": "ch1"}},
    }
    _, out_headers, _ = asyncio.run(
        get_payload(
            req,
            "openai",
            provider,
            api_key="sk-123",
            global_preferences={"user_agent": "CustomUA/1.0", "headers": {"X-Global": "g1"}},
        )
    )
    assert out_headers["User-Agent"] == "CustomUA/1.0"
    assert out_headers["X-Global"] == "g1"
    assert out_headers["X-Channel"] == "ch1"

