import json
import pathlib

from miora_plugin import BalancePlugin, BalanceQueryError, serve
from balance_common import CHAT_SUFFIXES, endpoint, number


class Plugin(BalancePlugin):
    def balance(self, context):
        url = endpoint(context.config["base_url"], "/user/balance", ("/user/balance", "/anthropic/v1/messages", "/anthropic/v1", "/anthropic", *CHAT_SUFFIXES))
        return self.normalize(self.read_balance(context, url))

    @staticmethod
    def normalize(data):
        rows = data.get("balance_infos", [])
        if not rows:
            raise BalanceQueryError("invalid_response")
        buckets = []
        for row in rows:
            currency = row.get("currency")
            total = number(row.get("total_balance"))
            if not currency or total is None:
                raise BalanceQueryError("invalid_response")
            if len(rows) > 1:
                buckets.append({"label": "总余额", "amount": total, "unit": currency})
            for field, label in (("granted_balance", "赠送余额"), ("topped_up_balance", "充值余额")):
                amount = number(row.get(field))
                if amount is not None:
                    buckets.append({"label": label, "amount": amount, "unit": currency})
        return {"available": data.get("is_available"),
            "amount": number(rows[0]["total_balance"]) if len(rows) == 1 else None,
            "currency": rows[0]["currency"] if len(rows) == 1 else None, "buckets": buckets}


if __name__ == "__main__":
    serve(Plugin(json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))))
