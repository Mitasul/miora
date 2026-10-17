import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest
import urllib.error
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "sdk" / "python"))
sys.path.insert(0, str(ROOT / "plugins" / "balances"))
import miora_plugin as sdk


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def plugin(name):
    directory = ROOT / "plugins" / "balances" / name
    module = load("balance_" + name.replace("-", "_"), directory / "plugin.py")
    return module.Plugin(json.loads((directory / "manifest.json").read_text(encoding="utf-8")))


def context(current, base=None, token="upstream-key", account="account-b", **config):
    defaults = {field["key"]: field["default"] for field in current.manifest.get("config_schema", []) if "default" in field}
    if base is not None:
        defaults["base_url"] = base
    defaults.update(config)
    defaults["api_key"] = token
    resolved = {"provider": "codex", "account_ref": "core-oauth:codex:b", "access_token": token, "account_id": account}
    return sdk.Context({}, defaults, "unchanged-channel", current.manifest["auth"], resolved_auth=resolved)


class PluginTests(unittest.TestCase):
    def request(self, name, data, base=None, **config):
        current = plugin(name)
        response = io.BytesIO(json.dumps(data).encode())
        transport = Mock()
        transport.open.return_value = response
        with patch.object(sdk.urllib.request, "build_opener", return_value=transport):
            value = current.query_balance(context(current, base, **config))
        self.assertEqual(transport.open.call_count, 1)
        self.assertEqual(transport.open.call_args.kwargs["timeout"], 6)
        request = transport.open.call_args.args[0]
        self.assertEqual(request.method, "GET")
        self.assertIsNone(request.data)
        self.assertEqual(request.get_header("Authorization"), "Bearer upstream-key")
        self.assertEqual(request.get_header("User-agent"), "Miora-Balance/1")
        return value, request

    def test_codex_keeps_plan_windows_credits_and_paired_core_account(self):
        value, request = self.request("codex", {"plan_type": "plus", "rate_limit": {
            "primary_window": {"used_percent": 25, "limit_window_seconds": 18000, "reset_at": 1900000000},
            "secondary_window": {"used_percent": 100, "limit_window_seconds": 604800}},
            "credits": {"balance": "12.5", "unlimited": False}}, "https://chatgpt.com/backend-api/codex")
        self.assertEqual(request.full_url, "https://chatgpt.com/backend-api/wham/usage")
        self.assertEqual(request.get_header("Chatgpt-account-id"), "account-b")
        self.assertEqual(value["plan"], "plus")
        self.assertEqual(value["windows"][0]["remaining_percent"], 75)
        self.assertEqual(value["windows"][1]["remaining_percent"], 0)
        self.assertEqual(value["amount"], 12.5)
        self.assertEqual(value["currency"], "credits")
        self.assertNotIn("refresh_token", json.dumps(value))

    def test_codex_additional_windows_and_unknown_values_remain_distinct(self):
        value = plugin("codex").normalize({"plan_type": "pro", "rate_limit": {"primary_window": {"reset_at": 1900000000}},
            "additional_rate_limits": [{"limit_name": "review", "rate_limit": {"secondary_window": {"used_percent": 0}}}]})
        self.assertIsNone(value["windows"][0]["remaining_percent"])
        self.assertEqual(value["windows"][1]["remaining_percent"], 100)
        self.assertIn("review", value["windows"][1]["label"])
        self.assertIsNone(value["amount"])

    def test_codex_does_not_access_a_refresh_store(self):
        current = plugin("codex")
        with patch.object(current, "read_balance", return_value={"plan_type": "plus"}), patch.object(sdk, "CredentialStore") as store:
            current.query_balance(context(current))
        store.assert_not_called()

    def test_novelai_subscription_and_purchased_anlas_keep_units_and_expiry(self):
        value, request = self.request("novelai", {"tier": 3, "active": True, "expiresAt": 1900000000,
            "trainingStepsLeft": {"fixedTrainingStepsLeft": 900, "purchasedTrainingSteps": 100}}, "https://api.novelai.net")
        self.assertEqual(request.full_url, "https://image.novelai.net/user/subscription")
        self.assertEqual(value["plan"], "Opus")
        self.assertEqual(value["amount"], 1000)
        self.assertEqual(value["currency"], "Anlas")
        self.assertEqual([bucket["amount"] for bucket in value["buckets"]], [900, 100])
        self.assertEqual(value["expires_at"], 1900000000)

    def test_novelai_missing_components_are_not_zero_or_unlimited(self):
        value = plugin("novelai").normalize({"tier": 12, "active": False, "trainingStepsLeft": {"fixedTrainingStepsLeft": 0}})
        self.assertEqual(value["plan"], "Tier 12")
        self.assertIsNone(value["amount"])
        self.assertNotIn("unlimited", value)
        self.assertEqual(value["buckets"][0]["amount"], 0)

    def test_deepseek_decimal_balances_are_not_a_subscription_or_usd_guess(self):
        value, request = self.request("deepseek", {"is_available": False, "balance_infos": [
            {"currency": "CNY", "total_balance": "0.00", "granted_balance": "0", "topped_up_balance": "0"}]}, "https://api.deepseek.com/v1")
        self.assertEqual(request.full_url, "https://api.deepseek.com/user/balance")
        self.assertEqual(value["amount"], 0)
        self.assertEqual(value["currency"], "CNY")
        self.assertFalse(value["available"])
        self.assertNotIn("active", value)
        self.assertNotIn("plan", value)

    def test_deepseek_preserves_each_currency_instead_of_adding_them(self):
        value = plugin("deepseek").normalize({"balance_infos": [
            {"currency": "CNY", "total_balance": "10", "granted_balance": "2"},
            {"currency": "USD", "total_balance": "5"}]})
        self.assertIsNone(value["amount"])
        self.assertEqual([(item["amount"], item["unit"]) for item in value["buckets"] if item["label"] == "总余额"], [(10, "CNY"), (5, "USD")])

    def test_deepseek_anthropic_protocol_uses_the_same_read_only_balance_endpoint(self):
        _, request = self.request("deepseek", {"balance_infos": [{"currency": "CNY", "total_balance": "1"}]}, "https://api.deepseek.com/anthropic")
        self.assertEqual(request.full_url, "https://api.deepseek.com/user/balance")

    def test_moonshot_wallet_separates_available_cash_and_vouchers(self):
        data = {"code": 0, "status": True, "data": {"available_balance": 20, "cash_balance": 15, "voucher_balance": 5}}
        for host, currency in [("api.moonshot.cn", "CNY"), ("api.moonshot.ai", "USD")]:
            value, request = self.request("moonshot", data, f"https://{host}/v1")
            self.assertEqual(request.full_url, f"https://{host}/v1/users/me/balance")
            self.assertEqual(value["amount"], 20)
            self.assertEqual(value["currency"], currency)
            self.assertEqual([item["amount"] for item in value["buckets"]], [15, 5])
            self.assertNotIn("plan", value)

    def test_moonshot_proxy_does_not_invent_a_currency(self):
        data = {"code": 0, "status": True, "data": {"available_balance": 0}}
        value, request = self.request("moonshot", data, "https://proxy.invalid/prefix/v1")
        self.assertIsNone(value["currency"])
        self.assertEqual(request.full_url, "https://proxy.invalid/prefix/v1/users/me/balance")
        value, _ = self.request("moonshot", data, "https://proxy.invalid/v1", currency="CNY")
        self.assertEqual(value["currency"], "CNY")

    def test_moonshot_http_200_business_errors_are_not_success(self):
        with self.assertRaises(sdk.BalanceQueryError) as raised:
            plugin("moonshot").normalize({"code": 401, "status": False, "data": {"available_balance": 100}})
        self.assertEqual(raised.exception.code, "upstream_error")

    def test_kimi_plan_detailed_quotas_use_reported_durations_and_iso_resets(self):
        value, request = self.request("kimi-plan", {"usage": {"limit": "100", "remaining": "75", "resetTime": "2026-10-18T00:00:00Z"},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"}, "detail": {"limit": "20", "used": "20", "resetTime": "2026-10-17T18:00:00+08:00"}}]})
        self.assertEqual(request.full_url, "https://api.kimi.com/coding/v1/usages")
        self.assertEqual(value["windows"][0]["remaining_percent"], 75)
        self.assertEqual(value["windows"][1]["remaining_percent"], 0)
        self.assertEqual(value["windows"][1]["limit_seconds"], 18000)
        self.assertEqual(value["windows"][1]["reset_at"], "2026-10-17T18:00:00+08:00")
        self.assertNotIn("amount", value)
        self.assertNotIn("currency", value)
        self.assertNotIn("plan", value)

    def test_kimi_plan_ratio_only_windows_are_not_silently_lost(self):
        value = plugin("kimi-plan").normalize({"usages": {"limit_5h": {"used_ratio": 0.25, "reset_time": "2026-10-18T00:00:00Z"},
            "month_total": {"used_ratio": 1}, "extra_limit": {"used_ratio": 0}}})
        self.assertEqual([item["remaining_percent"] for item in value["windows"]], [75, 0, 100])
        self.assertEqual(value["windows"][2]["label"], "extra_limit")

    def test_kimi_plan_detailed_windows_take_precedence_over_conflicting_ratios(self):
        value = plugin("kimi-plan").normalize({"usage": {"limit": 100, "used": 25}, "usages": {"limit_5h": {"used_ratio": 0.9}}})
        self.assertEqual(len(value["windows"]), 1)
        self.assertEqual(value["windows"][0]["remaining_percent"], 75)

    def test_kimi_plan_missing_denominator_does_not_invent_a_percent(self):
        value = plugin("kimi-plan").normalize({"usage": {"remaining": "40"}})
        self.assertIsNone(value["windows"][0]["remaining_percent"])
        self.assertEqual(value["windows"][0]["remaining"], 40)

    def test_kimi_plan_empty_or_error_payload_is_not_a_successful_query(self):
        for data in [{}, {"error": "denied"}, {"usage": {}, "limits": [{"detail": {}}]}]:
            with self.assertRaises(sdk.BalanceQueryError) as raised:
                plugin("kimi-plan").normalize(data)
            self.assertEqual(raised.exception.code, "invalid_response")

    def test_proxy_paths_are_preserved_without_sending_keys_to_a_different_origin(self):
        fixtures = [("codex", "https://proxy.invalid/prefix/backend-api/codex", {"plan_type": "pro"}, "https://proxy.invalid/prefix/backend-api/wham/usage"),
            ("novelai", "https://proxy.invalid/prefix/v1", {"tier": 3}, "https://proxy.invalid/prefix/user/subscription"),
            ("deepseek", "https://proxy.invalid/prefix/v1/chat/completions", {"balance_infos": [{"currency": "CNY", "total_balance": "1"}]}, "https://proxy.invalid/prefix/user/balance"),
            ("kimi-plan", "https://proxy.invalid/prefix/coding/v1/messages", {"usage": {"limit": 100, "remaining": 75}}, "https://proxy.invalid/prefix/coding/v1/usages")]
        for name, base, data, expected in fixtures:
            _, request = self.request(name, data, base)
            self.assertEqual(request.full_url, expected)

    def test_packaging_creates_five_uploadable_archives_without_credentials_or_sdk_copies(self):
        package = load("package_balances", ROOT / "scripts" / "package_balance_plugins.py")
        unpack = load("unpack_balances", ROOT / "src" / "plugins" / "managed_unpack.py")
        with tempfile.TemporaryDirectory() as directory:
            paths = package.package(pathlib.Path(directory))
            self.assertEqual(len(paths), 5)
            for path in paths:
                metadata = unpack.inspect_archive(path)
                self.assertTrue(metadata["manifest"]["capabilities"]["balance_query"])
                self.assertEqual(metadata["requirements"], [])


class BalanceSDKTests(unittest.TestCase):
    def setUp(self):
        self.plugin = plugin("deepseek")
        self.context = context(self.plugin)

    def test_http_errors_do_not_retry_and_do_not_return_raw_upstream_bodies(self):
        for status, expected in [(401, "access_denied"), (403, "access_denied"), (429, "rate_limited"), (302, "redirect_blocked"), (500, "upstream_error")]:
            transport = Mock()
            transport.open.side_effect = urllib.error.HTTPError("https://upstream.invalid", status, "private-secret", {}, io.BytesIO(b"private-token"))
            with patch.object(sdk.urllib.request, "build_opener", return_value=transport):
                with self.assertRaises(sdk.BalanceQueryError) as raised:
                    self.plugin.read_balance(self.context, "https://upstream.invalid")
            self.assertEqual(raised.exception.code, expected)
            self.assertNotIn("private", str(raised.exception))
            self.assertEqual(transport.open.call_count, 1)

    def test_redirect_transport_never_creates_a_followup_request(self):
        handler = sdk.NativeProxy.NoRedirect()
        self.assertIsNone(handler.redirect_request(None, None, 302, "redirect", {}, "https://different.invalid"))

    def test_proxy_and_current_account_headers_are_request_scoped(self):
        current = plugin("codex")
        for token, account, proxy in [("token-a", "account-a", "http://proxy-a.invalid"), ("token-b", "account-b", "http://proxy-b.invalid")]:
            request_context = context(current, token=token, account=account)
            request_context.request["proxy"] = proxy
            transport = Mock()
            transport.open.return_value = io.BytesIO(b'{"plan_type":"plus"}')
            with patch.object(sdk.urllib.request, "build_opener", return_value=transport) as build:
                current.query_balance(request_context)
            request = transport.open.call_args.args[0]
            self.assertEqual(request.get_header("Authorization"), "Bearer " + token)
            self.assertEqual(request.get_header("Chatgpt-account-id"), account)
            self.assertEqual(build.call_args.args[1].proxies, {"http": proxy, "https": proxy})

    def test_timeout_and_invalid_or_oversized_json_are_explicit_errors(self):
        for result, expected in [(TimeoutError(), "timeout"), (urllib.error.URLError("offline"), "network_error"),
            (io.BytesIO(b"not-json"), "invalid_response"), (io.BytesIO(b"x" * 262145), "response_too_large")]:
            transport = Mock()
            if isinstance(result, Exception):
                transport.open.side_effect = result
            else:
                transport.open.return_value = result
            with patch.object(sdk.urllib.request, "build_opener", return_value=transport):
                with self.assertRaises(sdk.BalanceQueryError) as raised:
                    self.plugin.read_balance(self.context, "https://upstream.invalid")
            self.assertEqual(raised.exception.code, expected)

    def test_cancelled_queries_never_open_a_connection(self):
        self.context.cancelled.set()
        with patch.object(sdk.urllib.request, "build_opener") as build:
            with self.assertRaises(InterruptedError):
                self.plugin.query_balance(self.context)
        build.assert_not_called()


if __name__ == "__main__":
    unittest.main()
