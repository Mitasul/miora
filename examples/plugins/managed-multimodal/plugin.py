import json
import pathlib
from miora_plugin import Channel, serve


manifest = json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))
serve(Channel(manifest))
