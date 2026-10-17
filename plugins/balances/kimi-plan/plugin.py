import json
import pathlib

from miora_plugin import BalancePlugin, BalanceQueryError, serve
from balance_common import endpoint, number


class Plugin(BalancePlugin):
    def balance(self, context):
        url = endpoint(context.config["base_url"], "/coding/v1/usages",
            ("/coding/v1/chat/completions", "/coding/v1/messages", "/coding/v1/usages", "/coding/v1", "/coding"))
        return self.normalize(self.read_balance(context, url))

    @staticmethod
    def normalize(data):
        windows = []
        if isinstance(data.get("usage"), dict):
            window = Plugin.window("订阅额度", data["usage"])
            if window is not None:
                windows.append(window)
        for position, item in enumerate(data.get("limits", []), 1):
            detail = item.get("detail", {})
            window = Plugin.window(item.get("name") or f"限额 {position}", detail)
            if window is None:
                continue
            period = item.get("window", {})
            multiplier = {"TIME_UNIT_SECOND": 1, "TIME_UNIT_MINUTE": 60, "TIME_UNIT_HOUR": 3600, "TIME_UNIT_DAY": 86400}.get(period.get("timeUnit"))
            duration = number(period.get("duration"))
            if multiplier is not None and duration is not None:
                window["limit_seconds"] = int(duration * multiplier)
            windows.append(window)
        if not windows:
            labels = {"limit_5h": "5 小时额度", "limit_7d": "7 天额度", "month_total": "月总额度", "month_code": "月代码额度"}
            for name, value in data.get("usages", {}).items():
                used = number(value.get("used_ratio")) if isinstance(value, dict) else None
                if used is not None:
                    windows.append({"label": labels.get(name, name), "used_percent": used * 100,
                        "remaining_percent": (1 - used) * 100, "reset_at": value.get("reset_time"), "limit_reached": used >= 1})
        if not windows:
            raise BalanceQueryError("invalid_response")
        return {"windows": windows}

    @staticmethod
    def window(label, detail):
        limit = number(detail.get("limit"))
        used = number(detail.get("used"))
        remaining = number(detail.get("remaining"))
        if limit is None and used is None and remaining is None and not detail.get("resetTime"):
            return None
        if used is None and limit is not None and remaining is not None:
            used = limit - remaining
        if limit is not None and used is not None:
            remaining = limit - used
        percentage = used / limit * 100 if used is not None and limit is not None and limit > 0 else None
        return {"label": label, "remaining": remaining, "used_percent": percentage,
            "remaining_percent": None if percentage is None else 100 - percentage,
            "reset_at": detail.get("resetTime"), "limit_reached": remaining <= 0 if remaining is not None else None}


if __name__ == "__main__":
    serve(Plugin(json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))))
