"""Verify retained upstream notices offline; this is not a legal clearance check."""
import argparse
import json
from pathlib import Path
import shutil
import time

from release_common import ROOT, digest

COMPONENTS = {"xasr", "sherpa", "onnxruntime", "whisper-runtime", "whisper-model", "silero"}
DOCUMENTS = ["THIRD_PARTY_NOTICES.md", "docs/LEGAL.md", "docs/sample-provenance.md"]


def license_materials(root=ROOT):
    root = Path(root).resolve()
    directory = root / "vendor/licenses"
    manifest = json.loads((directory / "SOURCES.json").read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise RuntimeError("Unsupported upstream notice inventory")
    records = manifest["files"]
    names = [record["name"] for record in records]
    if len(set(names)) != len(names):
        raise RuntimeError("Duplicate upstream notice entry")
    for record in records:
        name = record["name"]
        if name in ("", ".", "..", "SOURCES.json") or "/" in name or "\\" in name:
            raise RuntimeError("Unsafe upstream notice filename: " + name)
        path = directory / name
        if (not path.is_file() or path.stat().st_size != record["bytes"]
                or digest(path) != record["sha256"]):
            raise RuntimeError("Missing or altered upstream notice: " + name)
        if not record["source"].startswith("https://"):
            raise RuntimeError("Missing upstream source: " + name)
    actual = {path.name for path in directory.iterdir() if path.is_file()}
    if actual != {*names, "SOURCES.json"}:
        raise RuntimeError("Upstream notice inventory does not match vendor/licenses")
    components = manifest["components"]
    if len(components) != len(COMPONENTS) or {item["id"] for item in components} != COMPONENTS:
        raise RuntimeError("Incomplete model/runtime attribution inventory")
    for component in components:
        if not component["license_files"] or not component["attribution"]:
            raise RuntimeError("Missing attribution or license: " + component["id"])
        for name in component["license_files"] + component["evidence_files"]:
            if name not in names:
                raise RuntimeError("Unlisted attribution material: " + name)
        if component.get("artifact_manifest"):
            path = root / component["artifact_manifest"]
            if not path.resolve().is_relative_to(root):
                raise RuntimeError("Unsafe model manifest path")
            if digest(path) != component["artifact_manifest_sha256"]:
                raise RuntimeError("Model manifest changed; review licenses: " + component["id"])
    materials = [(directory / name, Path("licenses") / name) for name in [*names, "SOURCES.json"]]
    for name in DOCUMENTS:
        path = root / name
        if not path.is_file() or not path.stat().st_size:
            raise RuntimeError("Missing project disclosure: " + name)
        materials.append((path, Path(name)))
    return materials


def install_license_materials(destination, verify_only=False):
    """Keep notices alongside downloaded weights; preserve any differing old file."""
    materials = license_materials()
    destination = Path(destination).resolve()
    for source, relative in materials:
        target = destination / relative
        if not target.resolve().is_relative_to(destination):
            raise RuntimeError("Unsafe installed notice path: " + str(target))
        if target.is_file() and digest(target) == digest(source):
            continue
        if verify_only:
            raise RuntimeError("Missing or outdated installed notice: " + str(target))
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            target.rename(target.with_name(target.name + ".damaged-" + str(time.time_ns())))
        shutil.copy2(source, target)
    print("Upstream notices and project disclosures verified: " + str(destination), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installed-dir", type=Path,
                        help="Also check notices retained in this model/application directory")
    args = parser.parse_args()
    materials = license_materials()
    if args.installed_dir:
        install_license_materials(args.installed_dir, verify_only=True)
    print(json.dumps({"passed": True, "scope": "retained_notice_integrity_only",
                      "components": len(COMPONENTS), "files": len(materials)}, indent=2))


if __name__ == "__main__":
    main()
