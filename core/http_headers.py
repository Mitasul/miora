"""HTTP header 合并工具。"""


from typing import Optional


def has_header_case_insensitive(headers: Optional[dict], key: str) -> bool:
    """大小写无关地检查请求头是否存在。"""
    if not isinstance(headers, dict):
        return False

    key_lower = str(key).lower()
    return any(str(existing_key).lower() == key_lower for existing_key in headers.keys())


def _set_header_case_insensitive(headers: Optional[dict], key: str, value) -> None:
    """大小写无关地写入请求头，避免 Content-Type/content-type 一类重复键。"""
    if not isinstance(headers, dict):
        return

    key_str = str(key)
    key_lower = key_str.lower()
    target_key = None

    for existing_key in headers.keys():
        if str(existing_key).lower() == key_lower:
            target_key = existing_key
            break

    normalized_value = ",".join(str(i) for i in value) if isinstance(value, list) else str(value)

    if target_key is None:
        headers[key_str] = normalized_value
    else:
        headers[target_key] = normalized_value


def apply_custom_headers(headers: dict, custom_headers: Optional[dict]) -> None:
    """将自定义 headers 合并到请求头中。

    custom_headers 的值支持两种格式：
    - str: 直接设置
    - list[str]: 用逗号拼接后设置（符合 RFC 7230 §3.2.2）

    示例::
        {"anthropic-beta": ["val1", "val2"]}  →  "anthropic-beta": "val1,val2"
        {"X-Custom": "abc"}                   →  "X-Custom": "abc"
    """
    if not isinstance(custom_headers, dict):
        return
    for k, v in custom_headers.items():
        if v is None:
            continue
        # 值为 "null" 字符串时删除该 header（用于屏蔽渠道硬编码的头）
        if isinstance(v, str) and v.strip().lower() == "null":
            key_lower = str(k).lower()
            for existing_key in list(headers.keys()):
                if str(existing_key).lower() == key_lower:
                    del headers[existing_key]
                    break
            continue
        _set_header_case_insensitive(headers, k, v)


DEFAULT_OUTBOUND_USER_AGENT = "Miora"


def apply_global_outbound_headers(
    headers: dict,
    global_preferences: Optional[dict],
    *,
    is_passthrough: bool = False,
) -> None:
    """应用全局出站请求头与默认/自定义 User-Agent 标识。

    优先级：
    1. 若请求头中尚未显式携带 User-Agent（如渠道特定指纹或透传客户端 UA），
       在非透传模式下默认补齐 `User-Agent: Miora`。
    2. 若全局配置了 `user_agent`（非空字符串），覆盖或设置 `User-Agent`
       （值为 `"null"` 时移除 User-Agent）。
    3. 若全局配置了 `headers` / `custom_headers` 字典，按大小写无关合并到请求头中
       （渠道级 `preferences.headers` 在此之后执行，可进一步按渠道覆盖）。
    """
    if not isinstance(headers, dict):
        return
    prefs = global_preferences if isinstance(global_preferences, dict) else {}

    configured_ua = prefs.get("user_agent")
    if isinstance(configured_ua, str) and configured_ua.strip():
        ua_val = configured_ua.strip()
        if ua_val.lower() == "null":
            for existing_key in list(headers.keys()):
                if str(existing_key).lower() == "user-agent":
                    del headers[existing_key]
                    break
        else:
            _set_header_case_insensitive(headers, "User-Agent", ua_val)
    elif not is_passthrough and not has_header_case_insensitive(headers, "User-Agent"):
        headers["User-Agent"] = DEFAULT_OUTBOUND_USER_AGENT

    for key_name in ("headers", "custom_headers"):
        global_custom = prefs.get(key_name)
        if isinstance(global_custom, dict) and global_custom:
            apply_custom_headers(headers, global_custom)

