"""Download pinned ASR weights without building runtimes; Python 3.9+ stdlib only."""
import argparse
import json
from pathlib import Path
import shutil
import time

from asset_download import fetch, sha
from prepare_xasr import extract
from check_licenses import install_license_materials, license_materials

ROOT = Path(__file__).resolve().parents[1]
def verified(path, record):
    return (
        path.is_file()
        and path.stat().st_size == record["bytes"]
        and sha(path) == record["sha256"]
    )


def require_files(directory, records):
    for record in records:
        if not verified(directory / record["name"], record):
            raise RuntimeError("Missing or invalid model file: " + str(directory / record["name"]))


def install_verified(source, target, record):
    if verified(target, record):
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    pending = target.with_name(target.name + ".install-" + str(time.time_ns()))
    shutil.copy2(source, pending)
    require_files(pending.parent, [dict(record, name=pending.name)])
    if target.exists():
        target.rename(target.with_name(target.name + ".damaged-" + str(time.time_ns())))
    pending.replace(target)


def prepare_xasr(base, portable, verify_only):
    manifest = json.loads((ROOT / "assets/xasr-manifest.json").read_text(encoding="utf-8"))
    target = base / ("xasr-480ms" if portable else manifest["directory"])
    if not verify_only and not all(verified(target / item["name"], item) for item in manifest["files"]):
        archive = ROOT / "vendor/downloads/x-asr-480ms-punct-int8-research.tar.bz2"
        fetch(manifest["source"], archive, manifest["archive_bytes"], manifest["archive_sha256"])
        staging = ROOT / ".build/model-downloads"
        extract(archive, staging)
        source = staging / manifest["directory"]
        require_files(source, manifest["files"])
        for record in manifest["files"]:
            install_verified(source / record["name"], target / record["name"], record)
    require_files(target, manifest["files"])
    print("X-ASR weights verified: " + str(target), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=["xasr"], default="xasr", help="X-ASR 480 ms INT8 (~169 MB weights)")
    parser.add_argument("--portable-dir", type=Path,
                        help="Existing extracted app folder containing FastASR.exe and runtime/; default: source layout")
    parser.add_argument("--verify-only", action="store_true", help="Check sizes and SHA-256 without downloading")
    args = parser.parse_args()
    license_materials()
    portable = args.portable_dir is not None
    if portable:
        app = args.portable_dir.resolve()
        if not (app / "FastASR.exe").is_file() or not (app / "runtime").is_dir():
            parser.error("--portable-dir must contain FastASR.exe and runtime/")
        base = app / "models"
    else:
        base = ROOT / "vendor/models"
    install_license_materials(app if portable else base, verify_only=args.verify_only)
    prepare_xasr(base, portable, args.verify_only)
    print("Selected model files are ready. No runtime compilation was requested.", flush=True)


if __name__ == "__main__":
    main()
