"""Bounded lifecycle helpers for isolated local HTTP gateway fixtures."""
import http.client
from pathlib import Path
import subprocess
import sys
import time


def wait_for_gateway(process, port, timeout=10):
    """Fail immediately on child exit instead of repeatedly polling a dead port."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"gateway exited during startup (exit={process.returncode})")
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=min(1, max(0.01, deadline - time.monotonic())))
        try:
            conn.request("GET", "/healthz")
            response = conn.getresponse()
            response.read()
            if response.status == 200:
                return
        except (OSError, http.client.HTTPException):
            pass
        finally:
            conn.close()
        time.sleep(0.05)
    raise AssertionError(f"gateway startup exceeded {timeout}s")


def stop_gateway(process):
    """Always reap before attempting to inspect runtime logs."""
    if process.poll() is not None:
        process.wait()
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def report_gateway_failure(process, log, path):
    stop_gateway(process)
    log.flush()
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    print(text[-8000:], file=sys.stderr)
