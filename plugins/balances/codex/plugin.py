import json
import pathlib
import time

from miora_plugin import BalancePlugin, serve
from balance_common import CHAT_SUFFIXES, endpoint, number


class Plugin(BalancePlugin):
    def balance(self, context):
        url = endpoint(context.config["base_url"], "/backend-api/wham/usage",
            ("/backend-api/codex/responses", "/backend-api/codex", "/backend-api/wham/usage", *CHAT_SUFFIXES))
        return self.normalize(self.read_balance(context, url))

    @staticmethod
    def normalize(data):
        windows = []
        limits = [("", data.get("rate_limit", {}))]
        limits.extend((item.get("limit_name", "附加额度"), item.get("rate_limit", {})) for item in data.get("additional_rate_limits", []))
        for prefix, limits in limits:
            for name, label in (("primary_window", "短期额度"), ("secondary_window", "长期额度")):
                window = limits.get(name)
                if not isinstance(window, dict):
                    continue
                used = number(window.get("used_percent"))
                reset = window.get("reset_at")
                if reset is None and window.get("reset_after_seconds") is not None:
                    reset = int(time.time()) + window["reset_after_seconds"]
                windows.append({"label": f"{prefix} · {label}" if prefix else label,
                    "used_percent": used, "remaining_percent": None if used is None else 100 - used,
                    "reset_at": reset, "limit_seconds": window.get("limit_window_seconds"),
                    "limit_reached": limits.get("limit_reached")})
        credits = data.get("credits", {})
        amount = number(credits.get("balance"))
        return {"plan": data.get("plan_type"), "amount": amount, "currency": "credits" if amount is not None else None,
            "unlimited": credits.get("unlimited"), "windows": windows}


if __name__ == "__main__":
    serve(Plugin(json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))))
