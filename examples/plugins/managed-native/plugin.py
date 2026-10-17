import json
from pathlib import Path
from miora_plugin import NativeProxy, serve

manifest = json.loads(Path(__file__).with_name("manifest.json").read_text(encoding="utf-8"))
serve(NativeProxy(manifest))
