# SPDX-License-Identifier: Apache-2.0
"""Small HTTP transport for this sample; no buffering of whole chat responses."""
import http.client
import select
import socket
import threading
from urllib.parse import urlsplit

CHUNK_BYTES = 16 * 1024


class Cancelled(Exception):
    pass


class Upstream:
    """One downstream request owns one upstream connection, never pooled.

    The watcher reads no application bytes. After the complete request body has
    been consumed, a downstream FIN/reset shuts down the upstream socket even
    while the main thread is blocked waiting for response headers or an SSE byte.
    """
    def __init__(self, downstream, base_url, timeout):
        url = urlsplit(base_url)
        if (url.scheme not in ("http", "https") or not url.hostname or url.username
                or url.password or url.query or url.fragment):
            raise ValueError("base_url must be an HTTP(S) URL without credentials, query or fragment")
        cls = http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        self.connection = cls(url.hostname, url.port, timeout=min(timeout, 10))
        self.base_path = url.path.rstrip("/")
        self.timeout = timeout
        self.downstream = downstream
        self.cancelled = threading.Event()
        self.done = threading.Event()
        self.lock = threading.Lock()
        self.socket = None
        self.response = None
        self.watcher = threading.Thread(target=self._watch_disconnect, daemon=True)

    def __enter__(self):
        self.watcher.start()
        return self

    def _watch_disconnect(self):
        while not self.done.is_set():
            try:
                readable, _, _ = select.select([self.downstream], [], [], 0.05)
                if not readable:
                    continue
                # This service closes after each response; pipelining isn't
                # supported. Any byte after the consumed request is unexpected.
                self.downstream.recv(1, socket.MSG_PEEK)
            except OSError:
                pass
            self.cancelled.set()
            with self.lock:
                sock = self.socket
            if sock is not None:
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            return

    def request(self, method, suffix, body, headers):
        self.connection.connect()
        with self.lock:
            self.socket = self.connection.sock
            self.socket.settimeout(self.timeout)
            if self.cancelled.is_set():
                self.socket.shutdown(socket.SHUT_RDWR)
                raise Cancelled()
        self.connection.request(method, self.base_path + suffix, body=body, headers=headers)
        # Keep the socket reference above: http.client may detach connection.sock
        # for Connection: close, while HTTPResponse still owns the read side.
        self.response = self.connection.getresponse()
        return self.response

    def chunks(self):
        expected = self.response.length
        total = 0
        while True:
            if self.cancelled.is_set():
                raise Cancelled()
            block = self.response.read1(CHUNK_BYTES)
            if self.cancelled.is_set():
                raise Cancelled()
            if not block:
                if expected is not None and total != expected:
                    raise http.client.IncompleteRead(b"", expected - total)
                return
            total += len(block)
            yield block

    def read_bounded(self, limit):
        output = bytearray()
        for block in self.chunks():
            if len(output) + len(block) > limit:
                raise ValueError("upstream model directory exceeds limit")
            output.extend(block)
        return bytes(output)

    def __exit__(self, *_):
        self.done.set()
        self.watcher.join(timeout=0.2)
        if self.response is not None:
            self.response.close()
        self.connection.close()
        if self.socket is not None:
            self.socket.close()
