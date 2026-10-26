"""Package the reviewed offline app and source using an explicit file inventory."""
import argparse
import json
from pathlib import Path
import shutil
import zipfile
from release_common import ROOT, VERSION, NAME, REPORT_NAMES, UI_REPORT_NAMES, digest, source_files
from check_licenses import license_materials


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--require-ui", action="store_true", help="Require desktop input and pin-button regression reports")
    args = parser.parse_args()
    notices = license_materials()
    executable = ROOT / ".build/target/release/fast-asr.exe"
    executable_hash = digest(executable)
    report_dir = ROOT / "reports"
    reports = []
    for name in REPORT_NAMES + UI_REPORT_NAMES:
        path = report_dir / name
        if name in UI_REPORT_NAMES and not args.require_ui and not path.exists():
            continue
        report = json.loads(path.read_text(encoding="utf-8-sig"))
        if not report.get("passed") or report.get("version") != VERSION or report.get("executable_sha256") != executable_hash:
            raise RuntimeError(f"Report failed or does not match this executable: {name}; run scripts/verify.ps1")
        reports.append(path)

    out = ROOT / "dist" / NAME
    out.mkdir(parents=True, exist_ok=True)
    files = {}

    def copy(src, dest):
        src = Path(src)
        dest = Path(dest)
        if not src.is_absolute():
            src = ROOT / src
        target = out / dest
        if not target.resolve().is_relative_to(out.resolve()):
            raise RuntimeError("Unsafe package destination")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, target)
        files[dest.as_posix()] = {"bytes": target.stat().st_size, "sha256": digest(target)}

    copy(executable, "FastASR.exe")
    for name in ["whisper.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"]:
        copy(ROOT / "vendor/runtime/whisper-1.9.5-win-x64-cpu" / name, Path("runtime/whisper") / name)
    xasr = json.loads((ROOT / "assets/xasr-manifest.json").read_text(encoding="utf-8"))
    for name in ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"]:
        copy(ROOT / "vendor/runtime" / xasr["runtime"]["directory"] / "lib" / name, Path("runtime/sherpa") / name)
    whisper = json.loads((ROOT / "assets/whisper-manifest.json").read_text(encoding="utf-8"))
    for manifest, base, destination in [(xasr, ROOT / "vendor/models" / xasr["directory"], "models/xasr-480ms"), (whisper, ROOT / "vendor/models/whisper", "models")]:
        for file in manifest["files"]:
            src = base / file["name"]
            if src.stat().st_size != file["bytes"] or digest(src) != file["sha256"]:
                raise RuntimeError("Model hash mismatch: " + str(src))
            copy(src, Path(destination) / file["name"])
    vad = ROOT / "vendor/models/vad/ggml-silero-v5.1.2.bin"
    if digest(vad) != "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf":
        raise RuntimeError("VAD hash mismatch")
    copy(vad, "models/vad/ggml-silero-v5.1.2.bin")
    for name in ["whisper-manifest.json", "xasr-manifest.json"]:
        copy(Path("assets") / name, Path("models") / name)
    for file in (ROOT / "vendor/samples/asr").glob("*.wav"):
        copy(file, Path("samples") / file.name)
    for name in ["README.md", "LICENSE", "CHANGELOG.md"]:
        copy(name, name)
    for file, destination in notices:
        copy(file, destination)
    metadata = json.loads((ROOT / ".build/cargo-metadata.json").read_text(encoding="utf-8-sig"))
    inventory = []
    for crate in metadata["packages"]:
        if not crate.get("source"):
            continue
        inventory.append({key: crate.get(key) for key in ["name", "version", "license", "repository"]})
        for file in Path(crate["manifest_path"]).parent.iterdir():
            if file.is_file() and file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")):
                copy(file, Path("licenses/rust") / (crate["name"] + "-" + crate["version"]) / file.name)
    inventory_path = ROOT / ".build/rust-dependencies.json"
    inventory_path.write_text(json.dumps(inventory, indent=2), encoding="utf-8")
    copy(inventory_path, "licenses/rust-dependencies.json")
    for name in ["architecture.md", "usage.md", "validation.md", "release-review-0.3.1.md", "release-review-0.3.2.md", "release-review-0.3.3.md"]:
        copy(Path("docs") / name, Path("docs") / name)
    for file in (ROOT / "docs/images").glob("*.png"):
        copy(file, Path("docs/images") / file.name)
    for file in reports:
        copy(file, Path("docs/test-results") / file.name)
    info = {"version": VERSION, "default": "x-asr-zh-en-480ms", "models": ["x-asr-zh-en-480ms", "whisper-large-v3-turbo-q5_0"], "cpu_only": True,
            "desktop_regressions_verified": all((report_dir / name) in reports for name in UI_REPORT_NAMES), "executable_sha256": executable_hash, "files": files}
    (out / "build-info.json").write_text(json.dumps(info, indent=2), encoding="utf-8")
    archive = ROOT / "dist" / (NAME + ".zip")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=3, strict_timestamps=False) as bundle:
        for name in sorted([*files, "build-info.json"]):
            bundle.write(out / name, Path(NAME) / name)
    source = ROOT / "dist" / f"FastASR-{VERSION}-source.zip"
    with zipfile.ZipFile(source, "w", zipfile.ZIP_DEFLATED, compresslevel=6, strict_timestamps=False) as bundle:
        for file in source_files():
            bundle.write(file, file.relative_to(ROOT))
    checksums = ROOT / "dist" / f"FastASR-{VERSION}-SHA256SUMS.txt"
    checksums.write_text("\n".join(f"{digest(file)}  {file.name}" for file in [archive, source]) + "\n", encoding="utf-8")
    print(json.dumps({"directory": str(out), "archive": str(archive), "archive_bytes": archive.stat().st_size,
                      "source": str(source), "checksums": str(checksums), "uncompressed_bytes": sum(f["bytes"] for f in files.values())}, ensure_ascii=False, indent=2), flush=True)

if __name__ == "__main__":
    main()
