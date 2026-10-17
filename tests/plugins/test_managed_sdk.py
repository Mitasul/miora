import importlib.util
import io
import json
import os
import pathlib
import stat
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


load("miora_resources", "sdk/python/miora_resources.py")
sdk = load("miora_test_sdk", "sdk/python/miora_plugin.py")
unpack = load("miora_test_unpack", "src/plugins/managed_unpack.py")


def archive(extra=()):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as package:
        package.writestr("manifest.json", json.dumps({"id": "example", "version": "1.0.0"}))
        package.writestr("plugin.py", "from miora_plugin import OpenAIChat\n")
        for name, data in extra:
            if isinstance(name, str):
                raw_name = name
                name = zipfile.ZipInfo("entry")
                name.filename = raw_name
                name.orig_filename = raw_name
            package.writestr(name, data)
    buffer.seek(0)
    return buffer


class ManagedPackageTests(unittest.TestCase):
    def test_package_preview_does_not_execute_code_or_extract(self):
        result = unpack.inspect_archive(archive([("requirements.txt", "httpx==0.28.1\n")]))
        self.assertEqual(result["manifest"]["id"], "example")
        self.assertEqual(result["requirements"], ["httpx==0.28.1"])

    def test_package_extracts_only_to_new_package_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            target = pathlib.Path(directory) / "source"
            unpack.inspect_archive(archive([("nested/data.txt", "value")]), target)
            self.assertEqual((target / "nested/data.txt").read_text(), "value")
            with self.assertRaises(FileExistsError):
                unpack.inspect_archive(archive(), target)

    def test_unsafe_and_reserved_paths_are_rejected_before_extracting(self):
        for name in ["../outside", "/outside", "C:/outside", "a\\outside", ".venv/file", "__MIORA__/file"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                unpack.inspect_archive(archive([(name, "bad")]))

    def test_windows_aliases_cannot_overlap_or_target_devices(self):
        with patch.object(unpack.sys, "platform", "win32"):
            for name in ["MANIFEST.JSON", "nested/CON", "__miora__./file", "nested/file. "]:
                with self.subTest(name=name), self.assertRaises(ValueError):
                    unpack.inspect_archive(archive([(name, "bad")]))

    def test_archive_links_and_excess_entries_are_rejected(self):
        link = zipfile.ZipInfo("link")
        link.create_system = 3
        link.external_attr = (stat.S_IFLNK | 0o777) << 16
        with self.assertRaises(ValueError):
            unpack.inspect_archive(archive([(link, "../outside")]))
        with self.assertRaises(ValueError):
            unpack.inspect_archive(archive([(f"entry-{index}", "") for index in range(1023)]))

    def test_expanded_size_limit_does_not_need_to_allocate_the_expanded_payload(self):
        with zipfile.ZipFile(archive()) as package:
            package.infolist()[0].file_size = 65 * 1024 * 1024
            with patch.object(unpack.zipfile, "ZipFile", return_value=package), self.assertRaises(ValueError):
                unpack.inspect_archive(None)


class SDKTests(unittest.TestCase):
    def test_sse_tools_reasoning_usage_and_terminal_are_explicit(self):
        frames = list(sdk.sse_events([
            b": heartbeat\r\n", b"\r\n", b"event: response.reasoning_text.delta\n", b'data: {"type":"response.reasoning_text.delta","delta":"reason"}\n', b"\n",
            b'data: {"type":"response.output_item.added","item":{"type":"function_call","name":"lookup"}}\n', b"\n",
            b'data: {"type":"response.function_call_arguments.delta","delta":"{}"}\n', b"\n",
            b'data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":8}}}}\n', b"\n",
        ]))
        events = [event for kind, payload in frames for event in sdk.standard_events(kind, payload)]
        self.assertEqual([event["type"] for event in events], ["reasoning", "tool_start", "tool_delta", "usage", "done"])
        self.assertEqual(events[-2]["usage"]["input_tokens_details"]["cached_tokens"], 8)

    def test_chat_and_claude_events_keep_tools_and_usage(self):
        chat = list(sdk.standard_events("message", {"choices": [{"delta": {"content": "text", "reasoning_content": "think", "tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}}], "usage": {"prompt_tokens": 0}}))
        self.assertEqual([event["type"] for event in chat], ["text", "reasoning", "tool_delta", "usage"])
        claude = list(sdk.standard_events("content_block_delta", {"delta": {"type": "thinking_delta", "thinking": "reason"}}))
        self.assertEqual(claude[0]["type"], "reasoning")
        self.assertEqual(list(sdk.standard_events("message_stop", {})), [{"type": "done"}])

    def test_failed_stream_and_truncated_input_never_invent_completion(self):
        events = list(sdk.standard_events("response.failed", {"type": "response.failed", "response": {"error": {"message": "failed"}}}))
        self.assertEqual([event["type"] for event in events], ["error"])
        self.assertEqual([event["type"] for event in sdk.standard_events("response.completed", {"type":"response.completed","response":{"status":"failed"}})], ["error"])
        self.assertEqual(list(sdk.sse_events([b'data: {"type":"response.completed"}'])), [])

    def test_sdk_reuses_authentication_and_join_rules_without_network(self):
        context = sdk.Context({"body": {"model": "test"}}, {"base_url": "https://example.invalid/v1", "api_key": "secret"}, "channel", {"type": "api_key", "config_key": "api_key", "header": "x-api-key"})
        channel = sdk.Claude({})
        self.assertEqual(channel.url(context, "/v1/messages"), "https://example.invalid/v1/messages")
        self.assertEqual(channel.headers(context)["x-api-key"], "secret")
        self.assertEqual(channel.headers(context)["anthropic-version"], "2023-06-01")
        context.cancelled.set()
        with self.assertRaises(InterruptedError):
            channel.open_http(context, "/v1/messages", {})

    def test_credentials_survive_reload_and_remain_channel_isolated(self):
        with tempfile.TemporaryDirectory() as directory:
            first = sdk.CredentialStore("first/../channel", root=directory)
            second = sdk.CredentialStore("频道" * 128, root=directory)
            with first.transaction() as value:
                value.update(access_token="access", refresh_token="refresh", expires_at=123)
            self.assertEqual(sdk.CredentialStore("first/../channel", root=directory).status(), {"connected": True, "refreshable": True, "expires_at": 123})
            self.assertFalse(second.status()["connected"])
            before = first.path.stat().st_mtime_ns
            first.status()
            self.assertEqual(first.path.stat().st_mtime_ns, before)
            self.assertNotIn("access", json.dumps(first.status()))

    def test_oauth_rotation_is_persisted_and_failed_refresh_does_not_replace_tokens(self):
        class FakeOAuth(sdk.OAuth):
            def token_request(self, context, parameters):
                self.last_parameters = parameters
                return {"access_token": "new-access", "refresh_token": "rotated-refresh", "expires_in": 3600}

        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"MIORA_PLUGIN_STATE_DIR": directory}):
            context = sdk.Context({}, {"redirect_uri": "https://example.invalid/callback"}, "account", {})
            oauth = FakeOAuth("https://example.invalid/authorize", "https://example.invalid/token", "client", ["models"])
            authorization = oauth.authorize(context)
            with self.assertRaises(ValueError):
                oauth.exchange(context, "code", "wrong-state")
            self.assertTrue(oauth.exchange(context, "code", authorization["state"])["refreshable"])
            self.assertEqual(oauth.access_token(context, force=True), "new-access")
            self.assertEqual(oauth.last_parameters["refresh_token"], "rotated-refresh")
            before = context.credentials.path.read_text()
            with patch.object(oauth, "token_request", side_effect=RuntimeError("upstream failed")), self.assertRaises(RuntimeError):
                oauth.access_token(context, force=True)
            self.assertEqual(context.credentials.path.read_text(), before)


class MultimodalTests(unittest.TestCase):
    def context(self, request=None, schema=None):
        context = sdk.Context(request or {"body": {}, "endpoint": "/v1/images/generations", "model": "upstream"}, {"base_url": "https://example.invalid/v1", "api_key": "vendor-key"}, "images", {"type": "bearer"}, request_schema=schema or [])
        self.addCleanup(context.resources.close)
        return context

    def test_resources_are_scoped_bounded_and_removed_after_use(self):
        with sdk.ResourceStore(limit=8) as first, sdk.ResourceStore() as second:
            image = first.add(io.BytesIO(b"\x89PNG"), filename="../../image.png", content_type="image/png")
            self.assertEqual(image.filename, "image.png")
            self.assertEqual(b"".join(image.chunks()), b"\x89PNG")
            self.assertEqual(first.get(image.metadata()), image)
            self.assertNotIn("path", image.metadata())
            with self.assertRaises(sdk.RequestParameterError):
                second.get(image)
            with self.assertRaises(sdk.RequestParameterError):
                first.get({"resource_id": "../../outside"})
            with self.assertRaises(sdk.RequestParameterError):
                first.add(b"12345")
            first.add(b"1234")
        with self.assertRaises(sdk.RequestParameterError):
            image.open()

    def test_multipart_roundtrip_preserves_files_duplicates_and_boundary_like_bytes(self):
        context = self.context()
        payload = b"x" * 16300 + b"\x00\r\n--not-a-boundary\xff"
        image = context.resources.add(payload, filename="帧图.png", content_type="image/png")
        form = sdk.Multipart([("model", "alias"), ("vendorStyle", "line-art"), ("seed", "42"), ("image", image), ("tag", "one"), ("tag", "two")])
        original = b"".join(form)
        self.assertEqual(len(original), form.size)
        body = context.resources.add(original, resource_id="body", content_type=form.content_type)
        context.request.update(format="multipart", body_resource=body.metadata(), model="upstream")
        context.request_schema = [{"key": "seed", "type": "integer"}, {"key": "image", "type": "resource", "required": True}]
        values = context.parameters
        self.assertEqual(values["model"], "upstream")
        self.assertEqual(values["vendorStyle"], "line-art")
        self.assertEqual(values["seed"], 42)
        self.assertEqual(values["tag"], ["one", "two"])
        self.assertEqual(b"".join(values["image"].chunks()), payload)
        self.assertEqual(values["image"].filename, "帧图.png")
        self.assertIs(context.parameters, values)

    def test_invalid_and_empty_multipart_and_field_limits(self):
        from miora_resources import MultipartReader
        context = self.context()
        valid = sdk.Multipart({"prompt": "test"})
        with self.assertRaises(sdk.RequestParameterError):
            MultipartReader(io.BytesIO(b"".join(valid)[:-6]), valid.content_type).fields(context.resources)
        empty = sdk.Multipart({})
        self.assertEqual(MultipartReader(io.BytesIO(b"".join(empty)), empty.content_type).fields(context.resources), [])
        oversized = sdk.Multipart({"prompt": "x" * (1024 * 1024 + 1)})
        with self.assertRaises(sdk.RequestParameterError):
            MultipartReader(io.BytesIO(b"".join(oversized)), oversized.content_type).fields(context.resources)

    def test_false_boundary_prefix_in_binary_is_not_a_part_delimiter(self):
        from miora_resources import MultipartReader
        context = self.context()
        image = context.resources.add(b"head", filename="image")
        form = sdk.Multipart({"image": image})
        wire = b"".join(form).replace(b"head", b"head\r\n--" + form.boundary.encode() + b"XXtail")
        fields = MultipartReader(io.BytesIO(wire), form.content_type).fields(context.resources)
        self.assertEqual(b"".join(fields[0][1].chunks()), b"head\r\n--" + form.boundary.encode() + b"XXtail")

    def test_urlencoded_form_and_declared_types_are_plugin_owned(self):
        context = self.context(schema=[{"key": "enabled", "type": "boolean"}, {"key": "options", "type": "object"}])
        raw = b'model=alias&vendorStyle=ink&enabled=false&options=%7B%22quality%22%3A3%7D'
        body = context.resources.add(raw, resource_id="body", content_type="application/x-www-form-urlencoded")
        context.request.update(format="form", body_resource=body.metadata())
        self.assertEqual(context.parameters, {"model": "upstream", "vendorStyle": "ink", "enabled": False, "options": {"quality": 3}})

    def test_invalid_private_fields_fail_instead_of_being_silently_dropped(self):
        for field, value in [({"key": "seed", "type": "integer"}, True), ({"key": "style", "type": "string", "enum": ["ink"]}, "unknown"), ({"key": "image", "type": "resource"}, {"resource_id": "another-request"})]:
            context = self.context({"body": {field["key"]: value}}, [field])
            with self.subTest(field=field), self.assertRaises(sdk.RequestParameterError):
                context.parameters
        with self.assertRaises(sdk.RequestParameterError):
            self.context(schema=[{"key": "seed", "type": "integer", "required": True}]).parameters

    def test_http_json_form_multipart_and_binary_keep_auth_and_media_types(self):
        channel, context = sdk.Channel({}), self.context()
        resource = context.resources.add(b"image\x00data", content_type="image/png")
        captured = []
        def opened(request, **options):
            captured.append(request)
            return object()
        with patch.object(sdk.urllib.request, "urlopen", side_effect=opened):
            channel.open_http(context, "/v1/images/generations", {"vendorStyle": "ink"})
            channel.open_http(context, "/v1/images/generations", form={"prompt": "a b", "enabled": False, "options": {"seed": 42}})
            channel.open_http(context, "/v1/images/edits", multipart={"image": resource})
            channel.open_http(context, "/v1/images/edits", binary=resource, method="PUT")
        self.assertEqual(json.loads(captured[0].data), {"vendorStyle": "ink"})
        self.assertEqual(sdk.urllib.parse.parse_qs(captured[1].data.decode()), {"prompt": ["a b"], "enabled": ["false"], "options": ['{"seed": 42}']})
        self.assertEqual(captured[1].get_header("Content-type"), "application/x-www-form-urlencoded")
        multipart = b"".join(captured[2].data)
        self.assertEqual(int(captured[2].get_header("Content-length")), len(multipart))
        self.assertIn(b"image\x00data", multipart)
        self.assertEqual(b"".join(captured[3].data), b"image\x00data")
        self.assertEqual(captured[3].method, "PUT")
        self.assertEqual(captured[3].get_header("Content-type"), "image/png")
        self.assertTrue(all(request.get_header("Authorization") == "Bearer vendor-key" for request in captured))

    def test_binary_image_and_archive_responses_do_not_become_json(self):
        context = self.context()
        for content_type, payload in [("image/png", b"\x89PNG\x00"), ("application/zip", b"PK\x03\x04\x00")]:
            resource = context.resources.add(payload, filename="result.zip", content_type=content_type)
            response = sdk.Response.resource(resource, attachment=True)
            with response.body:
                self.assertEqual(response.body.read(), payload)
            self.assertEqual(response.headers["Content-Type"], content_type)
            self.assertIn("attachment", response.headers["Content-Disposition"])
        self.assertEqual(json.loads(sdk.Response.json({"task_id": "task-1"}, status=202).body), {"task_id": "task-1"})

    def test_task_polling_success_failure_timeout_and_cancellation(self):
        channel, context = sdk.Channel({}), self.context()
        with patch.object(channel, "open_http", return_value=object()), patch.object(channel, "read_json", side_effect=[{"status": "running"}, {"status": "succeeded", "url": "/result"}]), patch.object(context.cancelled, "wait", return_value=False):
            self.assertEqual(channel.poll_task(context, "/tasks/one")["url"], "/result")
        with patch.object(channel, "open_http", return_value=object()), patch.object(channel, "read_json", return_value={"status": "failed"}), self.assertRaises(RuntimeError):
            channel.poll_task(context, "/tasks/one")
        with patch.object(sdk.time, "monotonic", side_effect=[0, 2]), self.assertRaises(TimeoutError):
            channel.poll_task(context, "/tasks/one", timeout=1)
        context.cancelled.set()
        with patch.object(channel, "open_http") as opened, self.assertRaises(InterruptedError):
            channel.poll_task(context, "/tasks/one")
        opened.assert_not_called()

    def test_truncated_resource_transfer_is_an_explicit_error(self):
        with self.assertRaises(sdk.RequestParameterError):
            list(sdk.read_exact(io.BytesIO(b"short"), 8))

    def test_resource_envelope_decodes_binary_without_json_encoding_or_host_paths(self):
        context = self.context()
        raw = b"\x89PNG\x00\xff"
        envelope = {"request": {"format": "binary", "body_resource": {"resource_id": "body", "content_type": "image/png", "size": len(raw)}}}
        encoded = json.dumps(envelope).encode()
        headers = {"Content-Type": "application/vnd.miora.resource+octet-stream", "x-miora-envelope-length": str(len(encoded))}
        result = sdk.read_envelope(io.BytesIO(encoded + raw), len(encoded) + len(raw), headers, context.resources)
        self.assertEqual(result, envelope)
        self.assertEqual(b"".join(context.resources.get("body").chunks()), raw)
        with sdk.ResourceStore() as resources, self.assertRaises(sdk.RequestParameterError):
            sdk.read_envelope(io.BytesIO(encoded + raw[:-1]), len(encoded) + len(raw), headers, resources)
        with sdk.ResourceStore() as resources, self.assertRaises(sdk.RequestParameterError):
            sdk.read_envelope(io.BytesIO(encoded + raw), len(encoded) + len(raw) + 1, headers, resources)

    def test_mutable_defaults_are_not_shared_between_requests(self):
        schema = [{"key": "options", "type": "object", "default": {"style": "ink"}}]
        first, second = self.context(schema=schema), self.context(schema=schema)
        first.parameters["options"]["style"] = "photo"
        self.assertEqual(second.parameters["options"]["style"], "ink")

    def test_declared_query_parameters_are_typed_but_never_override_body_values(self):
        schema = [{"key": "seed", "type": "integer"}, {"key": "vendorStyle", "type": "string"}]
        context = self.context({"body": {"vendorStyle": "body-wins"}, "query_parameters": {"seed": "42", "vendorStyle": "query"}}, schema)
        self.assertEqual(context.parameters, {"seed": 42, "vendorStyle": "body-wins"})


class NativeProxyTests(unittest.TestCase):
    def context(self, raw=b'', content_type="application/json", method="POST"):
        context = sdk.Context({"format": "native", "method": method, "endpoint": "/native/generate", "query": "vendor=a%20b&vendor=c+d", "headers": [("content-type", content_type), ("vendor-beta", "experimental")]}, {"base_url": "https://vendor.invalid", "api_key": "managed-key"}, "native-channel", {"type": "bearer", "config_key": "api_key"}, request_schema=[{"key": "must-not-filter-native", "type": "integer", "required": True}])
        self.addCleanup(context.resources.close)
        context.request["body_resource"] = context.resources.add(raw, resource_id="body", content_type=content_type).metadata()
        return context

    def upstream(self, raw=b'', status=200):
        from email.message import Message
        response = io.BytesIO(raw)
        response.status = status
        response.headers = Message()
        response.headers["Content-Type"] = "application/octet-stream"
        response.headers["Set-Cookie"] = "one=first"
        response.headers["Set-Cookie"] = "two=second"
        return response

    def test_native_json_form_multipart_and_binary_are_not_reencoded_or_filtered(self):
        for content_type, raw in [("application/json", b' {"model":"native","tools":{"vendor":true}, "extra": [1, 2]} '), ("application/x-www-form-urlencoded", b"model=native&vendor=a+b&vendor=a%20b"), ("multipart/form-data; boundary=original", b"--original\r\nbinary\x00\xff\r\n--original--"), ("image/png", b"\x89PNG\x00\xff")]:
            with self.subTest(content_type=content_type):
                context = self.context(raw, content_type)
                channel = sdk.NativeProxy({})
                channel.transport = Mock()
                upstream = self.upstream(b"native-result", 202)
                channel.transport.open.return_value = upstream
                result = channel.invoke(context)
                request = channel.transport.open.call_args.args[0]
                headers = {name.lower(): value for name, value in request.header_items()}
                self.assertEqual(request.full_url, "https://vendor.invalid/native/generate?vendor=a%20b&vendor=c+d")
                self.assertEqual(headers["authorization"], "Bearer managed-key")
                self.assertEqual(headers["content-type"], content_type)
                self.assertEqual(headers["vendor-beta"], "experimental")
                self.assertEqual(b"".join(request.data), raw)
                self.assertEqual(result.status, 202)
                self.assertIs(result.body, upstream)
                self.assertEqual([value for name, value in result.headers if name == "Set-Cookie"], ["one=first", "two=second"])

    def test_native_query_authentication_uses_only_managed_upstream_credentials(self):
        context = self.context()
        context.authentication = {"type": "query", "query": "key", "config_key": "api_key"}
        context.config["api_key"] = "managed+/="
        channel = sdk.NativeProxy({})
        channel.transport = Mock()
        channel.transport.open.return_value = self.upstream()
        channel.invoke(context)
        request = channel.transport.open.call_args.args[0]
        self.assertTrue(request.full_url.endswith("vendor=a%20b&vendor=c+d&key=managed%2B%2F%3D"))
        self.assertFalse(request.has_header("Authorization"))

    def test_native_http_error_and_redirect_are_returned_not_followed_or_converted(self):
        for status in (302, 401, 422, 429, 503):
            context = self.context(method="GET")
            channel = sdk.NativeProxy({})
            channel.transport = Mock()
            error = sdk.urllib.error.HTTPError("https://vendor.invalid", status, "native", self.upstream().headers, io.BytesIO(b"native-error"))
            channel.transport.open.side_effect = error
            result = channel.invoke(context)
            self.assertEqual(result.status, status)
            self.assertIs(result.body, error)
            self.assertEqual(result.body.read(), b"native-error")
            self.assertEqual(channel.transport.open.call_args.args[0].get_method(), "GET")
            self.assertIsNone(channel.NoRedirect().redirect_request(None, None, status, "native", {}, "https://other.invalid"))

    def test_native_proxy_uses_existing_oauth_hook_and_refuses_protocol_conversion(self):
        context = self.context()
        channel = sdk.NativeProxy({})
        channel.transport = Mock()
        channel.transport.open.return_value = self.upstream()
        channel.oauth = Mock()
        channel.oauth.access_token.return_value = "refreshed-oauth"
        channel.invoke(context)
        self.assertEqual(channel.transport.open.call_args.args[0].get_header("Authorization"), "Bearer refreshed-oauth")
        context.request["format"] = "json"
        with self.assertRaises(sdk.RequestParameterError):
            channel.invoke(context)


class CoreOAuthTests(unittest.TestCase):
    def context(self, account="b"):
        return sdk.Context({}, {"base_url": "https://vendor.invalid", "account_id": "stale-account-a", "api_key": "not-the-token"}, "images",
            {"type": "core_oauth", "provider": "codex"}, resolved_auth={"provider": "codex", "account_ref": f"core-oauth:codex:{account}", "access_token": f"access-{account}", "account_id": f"account-{account}", "expires_at": 1900000000})

    def test_core_headers_pair_current_access_token_and_account_id(self):
        channel = sdk.Responses({})
        for account in ["a", "b", "a"]:
            context = self.context(account)
            headers = channel.headers(context)
            self.assertEqual(headers["Authorization"], f"Bearer access-{account}")
            self.assertEqual(headers["ChatGPT-Account-Id"], f"account-{account}")
            self.assertEqual(context.account["account_ref"], f"core-oauth:codex:{account}")
            self.assertNotIn("access_token", context.account)
            self.assertNotIn("refresh_token", context.resolved_auth)

    def test_core_credentials_cannot_use_local_store_or_plugin_owned_oauth(self):
        context = self.context()
        with patch.object(sdk, "CredentialStore") as store:
            with self.assertRaisesRegex(ValueError, "managed by Miora"):
                context.credentials
            store.assert_not_called()
        channel = sdk.Responses({})
        channel.oauth = Mock()
        with self.assertRaisesRegex(ValueError, "cannot refresh"):
            channel.headers(context)
        channel.oauth.access_token.assert_not_called()

    def test_missing_or_wrong_supplier_auth_never_falls_back_to_config_credentials(self):
        for resolved in [{}, {"provider": "claude", "access_token": "other", "account_id": "other"}, {"provider": "codex", "access_token": "access"}]:
            context = self.context()
            context.resolved_auth = resolved
            with patch.object(sdk.urllib.request, "urlopen") as opened:
                with self.assertRaisesRegex(ValueError, "not resolved"):
                    sdk.Responses({}).open_http(context, "/v1/responses", body={})
                opened.assert_not_called()

    def test_native_proxy_overrides_stale_account_header_with_selected_account(self):
        context = self.context()
        context.request = {"format": "native", "method": "POST", "endpoint": "/images", "headers": [["chatgpt-account-id", "stale-account-a"]], "body_resource": {"resource_id": "body", "content_type": "image/png", "size": 4}}
        context.resources.add(b"\x89PNG", content_type="image/png", resource_id="body")
        self.addCleanup(context.resources.close)
        channel = sdk.NativeProxy({})
        channel.transport = Mock()
        channel.transport.open.return_value = NativeProxyTests.upstream(self)
        channel.invoke(context)
        request = channel.transport.open.call_args.args[0]
        headers = {name.lower(): value for name, value in request.header_items()}
        self.assertEqual(headers["authorization"], "Bearer access-b")
        self.assertEqual(headers["chatgpt-account-id"], "account-b")
        self.assertEqual(b"".join(request.data), b"\x89PNG")

    def test_model_discovery_uses_the_same_resolved_authentication(self):
        channel = sdk.Responses({"capabilities": {"model_discovery": True}})
        response = io.BytesIO(b'{"data":[{"id":"image-model"}]}')
        response.status = 200
        with patch.object(sdk.urllib.request, "urlopen", return_value=response) as opened:
            models = channel.models(self.context())
        self.assertEqual(models, [{"id": "image-model", "name": "image-model"}])
        headers = {name.lower(): value for name, value in opened.call_args.args[0].header_items()}
        self.assertEqual(headers["authorization"], "Bearer access-b")
        self.assertEqual(headers["chatgpt-account-id"], "account-b")


class BalanceTests(unittest.TestCase):
    def test_balance_hook_reuses_resolved_core_account_without_a_refresh_store(self):
        context = sdk.Context({}, {}, "images", {"type": "core_oauth", "provider": "codex"},
            resolved_auth={"provider": "codex", "account_ref": "core-oauth:codex:b", "access_token": "access-b", "account_id": "account-b"})

        class Images(sdk.Responses):
            def balance(self, current):
                self.captured = (current.account, self.headers(current))
                return {"plan": "plus", "windows": [{"label": "5h", "remaining_percent": 75}]}

        channel = Images({"capabilities": {"balance_query": True}})
        with patch.object(sdk, "CredentialStore") as store:
            result = channel.query_balance(context)
            store.assert_not_called()
        self.assertEqual(result["plan"], "plus")
        self.assertEqual(channel.captured[1]["Authorization"], "Bearer access-b")
        self.assertEqual(channel.captured[1]["ChatGPT-Account-Id"], "account-b")
        self.assertNotIn("access_token", channel.captured[0])

    def test_undeclared_balance_hook_is_not_executed(self):
        channel = sdk.OpenAIChat({"capabilities": {}})
        channel.balance = Mock()
        with self.assertRaisesRegex(sdk.RequestParameterError, "balance_query"):
            channel.query_balance(sdk.Context({}, {}, "images", {}))
        channel.balance.assert_not_called()

    def test_declared_but_missing_balance_hook_does_not_return_fake_success(self):
        channel = sdk.OpenAIChat({"capabilities": {"balance_query": True}})
        with self.assertRaisesRegex(NotImplementedError, "balance query"):
            channel.query_balance(sdk.Context({}, {}, "images", {}))


if __name__ == "__main__":
    unittest.main()
