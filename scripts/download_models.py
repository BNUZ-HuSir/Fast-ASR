"""Download pinned ASR weights without building runtimes; Python 3.9+ stdlib only."""
import argparse
import json
from pathlib import Path
import shutil
import time

from prepare_whisper import fetch, sha
from prepare_xasr import extract
from check_licenses import install_license_materials, license_materials

ROOT = Path(__file__).resolve().parents[1]
VAD_NAME = "ggml-silero-v5.1.2.bin"
VAD_BYTES = 885098
VAD_SHA256 = "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf"
VAD_URL = (
    "https://huggingface.co/ggml-org/whisper-vad/resolve/"
    "9ffd54a1e1ee413ddf265af9913beaf518d1639b/" + VAD_NAME
)


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


def prepare_turbo(base, portable, verify_only):
    manifest = json.loads((ROOT / "assets/whisper-manifest.json").read_text(encoding="utf-8"))
    target = base if portable else base / "whisper"
    for record in manifest["files"]:
        if not verify_only:
            url = manifest["repository"] + "/resolve/" + manifest["revision"] + "/" + record["name"]
            fetch(url, target / record["name"], record["bytes"], record["sha256"])
    require_files(target, manifest["files"])
    vad = {"name": VAD_NAME, "bytes": VAD_BYTES, "sha256": VAD_SHA256}
    if not verify_only:
        fetch(VAD_URL, base / "vad" / VAD_NAME, VAD_BYTES, VAD_SHA256)
    require_files(base / "vad", [vad])
    print("Whisper Turbo and VAD weights verified: " + str(target), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=["xasr", "turbo", "all"], default="xasr",
                        help="Default: xasr (~169 MB weights); turbo is ~574 MB plus VAD")
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
    if args.model in ("xasr", "all"):
        prepare_xasr(base, portable, args.verify_only)
    if args.model in ("turbo", "all"):
        prepare_turbo(base, portable, args.verify_only)
    print("Selected model files are ready. No runtime compilation was requested.", flush=True)


if __name__ == "__main__":
    main()
