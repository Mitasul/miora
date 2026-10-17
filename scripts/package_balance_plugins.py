import argparse
import json
import pathlib
import zipfile


def package(output):
    source = pathlib.Path(__file__).resolve().parents[1] / "plugins" / "balances"
    output.mkdir(parents=True, exist_ok=True)
    paths = []
    for manifest_file in sorted(source.glob("*/manifest.json")):
        manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
        destination = output / f"{manifest['id']}-{manifest['version']}.zip"
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.write(manifest_file, "manifest.json")
            archive.write(manifest_file.parent / "plugin.py", "plugin.py")
            archive.write(source / "balance_common.py", "balance_common.py")
        paths.append(destination)
    return paths


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1] / ".artifacts" / "plugins")
    for path in package(parser.parse_args().output):
        print(path)
