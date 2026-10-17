import json
import pathlib
from urllib.parse import urlsplit, urlunsplit

from miora_plugin import BalancePlugin, serve
from balance_common import CHAT_SUFFIXES, endpoint, number


class Plugin(BalancePlugin):
    def balance(self, context):
        url = endpoint(context.config["base_url"], "/user/subscription",
            ("/ai/generate-image", "/user/subscription", "/v1/images/generations", *CHAT_SUFFIXES))
        parts = urlsplit(url)
        if parts.hostname in ("api.novelai.net", "text.novelai.net"):
            url = urlunsplit((parts.scheme, "image.novelai.net", parts.path, "", ""))
        return self.normalize(self.read_balance(context, url))

    @staticmethod
    def normalize(data):
        tier = data.get("tier")
        plans = {0: "Paper", 1: "Tablet", 2: "Scroll", 3: "Opus"}
        balance = data.get("trainingStepsLeft", {})
        fixed = number(balance.get("fixedTrainingStepsLeft"))
        purchased = number(balance.get("purchasedTrainingSteps"))
        buckets = [{"label": label, "amount": amount, "unit": "Anlas"} for label, amount in
            (("订阅 Anlas", fixed), ("购买 Anlas", purchased)) if amount is not None]
        return {"plan": None if tier is None else plans.get(tier, f"Tier {tier}"),
            "active": data.get("active"), "expires_at": data.get("expiresAt") or None,
            "amount": fixed + purchased if fixed is not None and purchased is not None else None,
            "currency": "Anlas", "buckets": buckets}


if __name__ == "__main__":
    serve(Plugin(json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))))
