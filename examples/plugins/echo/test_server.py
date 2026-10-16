#!/usr/bin/env python3
"""No dependencies: python -m unittest discover -s examples/plugins/echo -v"""
import contextlib
import io
import json
import threading
import unittest
import urllib.error
import urllib.request

from server import Handler, MANIFEST, ThreadingHTTPServer


class PluginServiceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.server.token = "test-service-token"
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.origin = "http://127.0.0.1:%s" % cls.server.server_port

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(3)

    def request(self, path, payload=None, authenticated=True):
        headers = {"Content-Type": "application/json"}
        if authenticated:
            headers["Authorization"] = "Bearer test-service-token"
        data = json.dumps(payload).encode() if payload is not None else None
        req = urllib.request.Request(self.origin + path, data=data, headers=headers)
        return urllib.request.urlopen(req, timeout=5)

    def envelope(self, endpoint="/v1/chat/completions", stream=False, prefix="Echo"):
        return {"api_version": 1, "plugin": {"id": "echo", "version": "1.0.0", "revision": 7},
                "request": {"id": "test", "endpoint": endpoint, "model": "echo-model", "method": "POST", "stream": stream,
                            "body": {"model": "echo-model", "messages": [{"role": "user", "content": "hello"}], "input": "hello"}},
                "channel": {"name": "demo", "config": {"prefix": prefix, "delay_ms": 0, "credential": "never-echo-this"}},
                "authentication": {"type": "none"}}

    def test_independent_control_authentication(self):
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.request("/health", authenticated=False)
        self.assertEqual(caught.exception.code, 401)
        with self.request("/health") as response:
            self.assertTrue(json.load(response)["ok"])

    def test_model_discovery_and_manifest(self):
        with self.request("/manifest") as response:
            self.assertEqual(json.load(response), MANIFEST)
        with self.request("/models", self.envelope()) as response:
            self.assertEqual(json.load(response)["models"][0]["id"], "echo-model")

    def test_json_channel_execution_and_secret_not_echoed(self):
        with self.request("/execute", self.envelope(prefix="Revision one")) as response:
            text = response.read().decode()
        self.assertNotIn("never-echo-this", text)
        self.assertEqual(json.loads(text)["choices"][0]["message"]["content"], "Revision one: hello")

    def test_chat_sse_execution(self):
        with self.request("/execute", self.envelope(stream=True)) as response:
            self.assertEqual(response.headers["Content-Type"], "text/event-stream")
            self.assertEqual(response.headers["X-Demo-Revision"], "7")
            text = response.read().decode()
        self.assertIn('"delta":{"content":', text)
        self.assertTrue(text.endswith("data: [DONE]\n\n"))

    def test_responses_json_and_sse(self):
        with self.request("/execute", self.envelope("/v1/responses")) as response:
            self.assertEqual(json.load(response)["status"], "completed")
        with self.request("/execute", self.envelope("/v1/responses", stream=True)) as response:
            text = response.read().decode()
        self.assertIn("event: response.output_text.delta", text)
        self.assertIn("event: response.completed", text)

    def test_embeddings(self):
        with self.request("/execute", self.envelope("/v1/embeddings")) as response:
            self.assertEqual(len(json.load(response)["data"][0]["embedding"]), 3)

    def test_failure_status_and_bad_contract(self):
        request = self.envelope()
        request["channel"]["config"]["fail_status"] = 503
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.request("/execute", request)
        self.assertEqual(caught.exception.code, 503)
        request["api_version"] = 99
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.request("/execute", request)
        self.assertEqual(caught.exception.code, 400)


if __name__ == "__main__":
    unittest.main()
