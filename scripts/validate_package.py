"""Check release hashes, inventory, source completeness and Unicode extraction."""
import hashlib
import json
import zipfile
from release_common import ROOT, VERSION, NAME, digest


def digest_stream(stream):
    result = hashlib.sha256()
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        result.update(block)
    return result.hexdigest()


def main():
    checks = {}
    destination = ROOT / ".build" / ("交付验证-" + VERSION.replace(".", ""))
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(ROOT / "dist" / (NAME + ".zip")) as bundle:
        checks["portable_crc"] = bundle.testzip() is None
        info = json.loads(bundle.read(NAME + "/build-info.json"))
        checks["version"] = info["version"] == VERSION
        checks["exact_inventory"] = set(bundle.namelist()) == {NAME + "/" + name for name in [*info["files"], "build-info.json"]}
        hashes = True
        for name, record in info["files"].items():
            with bundle.open(NAME + "/" + name) as stream:
                if bundle.getinfo(NAME + "/" + name).file_size != record["bytes"] or digest_stream(stream) != record["sha256"]:
                    hashes = False
        checks["portable_hashes"] = hashes
        checks["default_xasr"] = info["default"] == "x-asr-zh-en-480ms"
        required = ["models/xasr-480ms/encoder.int8.onnx", "models/xasr-480ms/decoder.onnx", "models/xasr-480ms/joiner.int8.onnx", "models/ggml-large-v3-turbo-q5_0.bin", "LICENSE"]
        checks["both_models_and_license"] = all(NAME + "/" + name in bundle.namelist() for name in required)
        checks["retired_weights_absent"] = not any("ggml-small" in name or "2023-02-" in name for name in bundle.namelist())
        checks["no_user_history_bundled"] = not any(name.startswith((NAME + "/data/", NAME + "/reports/")) for name in bundle.namelist())
        for name in bundle.namelist():
            if not (destination / name).resolve().is_relative_to(destination.resolve()):
                raise RuntimeError("Unsafe ZIP path")
        bundle.extractall(destination)
    with zipfile.ZipFile(ROOT / "dist" / f"FastASR-{VERSION}-source.zip") as bundle:
        checks["source_crc"] = bundle.testzip() is None
        required = ["LICENSE", "Cargo.lock", "src/xasr_bridge.cpp", "scripts/prepare_assets.py", "scripts/verify.ps1", "scripts/prepare_whisper.py", ".github/workflows/windows.yml", "vendor/samples/asr/46.wav"]
        checks["source_build_inputs"] = all(name in bundle.namelist() for name in required)
        checks["source_no_caches_or_secrets"] = not any(name.startswith((".build/", "target/", "reports/", "data/", "dist/")) or name.endswith((".exe", ".dll", ".bin", ".onnx", ".lib", ".exp")) or ".env" in name.split("/") for name in bundle.namelist())
    checks["release_checksums"] = True
    for line in (ROOT / "dist" / f"FastASR-{VERSION}-SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        checks["release_checksums"] &= digest(ROOT / "dist" / name) == expected
    report = ROOT / "reports" / ("package-integrity-" + VERSION + ".json")
    report.parent.mkdir(parents=True, exist_ok=True)
    value = {"passed": all(checks.values()), "checks": checks, "extracted": str(destination / NAME)}
    report.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(value, ensure_ascii=False, indent=2), flush=True)
    if not value["passed"]:
        raise SystemExit(1)

if __name__ == "__main__":
    main()
