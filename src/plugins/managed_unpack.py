import json
import pathlib
import stat
import sys
import zipfile


def inspect_archive(archive, destination=None):
    with zipfile.ZipFile(archive) as package:
        entries = package.infolist()
        if len(entries) > 1024 or sum(entry.file_size for entry in entries) > 64 * 1024 * 1024:
            raise ValueError("Plugin package exceeds 1024 entries or 64 MiB expanded")
        names = set()
        identities = set()
        for entry in entries:
            name = entry.orig_filename
            path = pathlib.PurePosixPath(name)
            if not name or len(name) > 1024 or "\0" in name or "\\" in name or ":" in name or path.is_absolute() or ".." in path.parts:
                raise ValueError("Plugin archive contains an unsafe path")
            if path.parts and path.parts[0].casefold() in {".venv", "__miora__"}:
                raise ValueError("Plugin archive uses a reserved runtime directory")
            if sys.platform == "win32" and any(part.endswith((".", " ")) or pathlib.PureWindowsPath(part).is_reserved() for part in path.parts):
                raise ValueError("Plugin archive contains an unsupported Windows path")
            identity = str(path).casefold() if sys.platform == "win32" else str(path)
            if identity in identities:
                raise ValueError("Plugin archive contains overlapping paths")
            identities.add(identity)
            if stat.S_ISLNK(entry.external_attr >> 16) or name in names:
                raise ValueError("Plugin archive contains a link or duplicate path")
            names.add(name)
        if "manifest.json" not in names or "plugin.py" not in names:
            raise ValueError("Plugin package requires manifest.json and plugin.py at its root")
        if package.getinfo("manifest.json").file_size > 256 * 1024:
            raise ValueError("Plugin manifest exceeds 256 KiB")
        manifest = json.loads(package.read("manifest.json"))
        requirements = []
        if "requirements.txt" in names:
            if package.getinfo("requirements.txt").file_size > 64 * 1024:
                raise ValueError("Plugin requirements exceed 64 KiB")
            requirements = package.read("requirements.txt").decode("utf-8").splitlines()
        if destination is not None:
            root = pathlib.Path(destination)
            resolved_root = root.resolve()
            for entry in entries:
                target = root.joinpath(*pathlib.PurePosixPath(entry.filename).parts)
                if not target.resolve().is_relative_to(resolved_root):
                    raise ValueError("Plugin archive paths overlap or escape the package directory")
                if entry.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with package.open(entry) as source, target.open("xb") as output:
                        while chunk := source.read(65536):
                            output.write(chunk)
        return {"manifest": manifest, "requirements": requirements, "kind": "managed"}


if __name__ == "__main__":
    try:
        result = inspect_archive(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
        print(json.dumps(result, ensure_ascii=True))
    except (OSError, ValueError, zipfile.BadZipFile, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
