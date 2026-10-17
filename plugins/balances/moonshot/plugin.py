import json
import pathlib
from urllib.parse import urlsplit

from miora_plugin import BalancePlugin, BalanceQueryError, serve
from balance_common import CHAT_SUFFIXES, endpoint, number


class Plugin(BalancePlugin):
    def balance(self, context):
        base = context.config["base_url"]
        url = endpoint(base, "/v1/users/me/balance", ("/v1/users/me/balance", *CHAT_SUFFIXES))
        currency = context.config.get("currency") or {"api.moonshot.cn": "CNY", "api.moonshot.ai": "USD"}.get(urlsplit(base).hostname)
        return self.normalize(self.read_balance(context, url), currency)

    @staticmethod
    def normalize(data, currency=None):
        if data.get("code") != 0 or data.get("status") is not True:
            raise BalanceQueryError("upstream_error")
        balance = data.get("data", {})
        amount = number(balance.get("available_balance"))
        if amount is None:
            raise BalanceQueryError("invalid_response")
        buckets = []
        for field, label in (("cash_balance", "现金余额"), ("voucher_balance", "代金券余额")):
            value = number(balance.get(field))
            if value is not None:
                buckets.append({"label": label, "amount": value, "unit": currency or ""})
        return {"amount": amount, "currency": currency, "buckets": buckets}


if __name__ == "__main__":
    serve(Plugin(json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))))
