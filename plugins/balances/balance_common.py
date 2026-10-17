import math
from urllib.parse import urlsplit, urlunsplit


def number(value):
    if value is None or isinstance(value, bool):
        return None
    try:
        value = float(value)
    except (TypeError, ValueError):
        return None
    return value if math.isfinite(value) else None


def endpoint(base, path, suffixes):
    url = urlsplit(base)
    prefix = url.path.rstrip("/")
    for suffix in suffixes:
        if prefix.endswith(suffix):
            prefix = prefix[:-len(suffix)]
            break
    return urlunsplit((url.scheme, url.netloc, prefix + path, "", ""))


CHAT_SUFFIXES = ("/v1/chat/completions", "/v1/responses", "/chat/completions", "/v1")
