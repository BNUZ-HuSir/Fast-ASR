"""Check release hashes, inventory, source completeness and Unicode extraction."""
import hashlib
import json
import zipfile
from release_common import ROOT, VERSION, NAME, digest, REPORT_NAMES, UI_REPORT_NAMES
from check_licenses import license_materials


def digest_stream(stream):
    result = hashlib.sha256()
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        result.update(block)
    return result.hexdigest()


def main():
    checks = {}
    notices = license_materials()
    destination = ROOT / ".build" / ("交付验证-" + VERSION.replace(".", ""))
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(ROOT / "dist" / (NAME + ".zip")) as bundle:
        checks["portable_crc"] = bundle.testzip() is None
        info = json.loads(bundle.read(NAME + "/build-info.json"))
        checks["version"] = info["version"] == VERSION
        checks["verification_disclosure"] = (
            info["desktop_regressions_verified"] and not info["desktop_tests_pending"]
            or info["release_channel"] == "preview" and bool(info["validation_limitations"])
            and set(info["desktop_tests_pending"]) == set(UI_REPORT_NAMES))
        checks["matching_verification_reports"] = all(
            (lambda report: report.get("passed") is True and report.get("version") == VERSION
             and report.get("executable_sha256") == info["executable_sha256"])(
                 json.loads(bundle.read(NAME + "/docs/test-results/" + name)))
            for name in REPORT_NAMES)
        lifecycle = json.loads(bundle.read(NAME + "/docs/test-results/hook-lifecycle.json"))
        checks["native_hook_shutdown"] = lifecycle.get("process_exit_signaled") is True and lifecycle.get("private_desktop") is True
        checks["exact_inventory"] = set(bundle.namelist()) == {NAME + "/" + name for name in [*info["files"], "build-info.json"]}
        hashes = True
        for name, record in info["files"].items():
            with bundle.open(NAME + "/" + name) as stream:
                if bundle.getinfo(NAME + "/" + name).file_size != record["bytes"] or digest_stream(stream) != record["sha256"]:
                    hashes = False
        checks["portable_hashes"] = hashes
        checks["default_xasr"] = info["default"] == "x-asr-zh-en-480ms"
        required = ["models/xasr-480ms/encoder.int8.onnx", "models/xasr-480ms/decoder.onnx", "models/xasr-480ms/joiner.int8.onnx", "LICENSE"]
        checks["xasr_and_license"] = all(NAME + "/" + name in bundle.namelist() for name in required)
        checks["upstream_notices_and_disclosures"] = all(
            NAME + "/" + relative.as_posix() in bundle.namelist()
            and hashlib.sha256(bundle.read(NAME + "/" + relative.as_posix())).hexdigest() == digest(source)
            for source, relative in notices
        )
        checks["only_xasr_model"] = info["models"] == ["x-asr-zh-en-480ms"]
        checks["retired_weights_and_runtimes_absent"] = not any("ggml-" in name or "/models/vad/" in name or "/runtime/whisper/" in name or "2023-02-" in name for name in bundle.namelist())
        checks["vad_removed"] = info["external_vad"] is False
        checks["developer_credit"] = info["developer"] == "Elza"
        checks["default_left_alt_hold"] = info["first_use_shortcut"] == "Left Alt hold 500 ms"
        checks["no_user_history_bundled"] = not any(name.startswith((NAME + "/data/", NAME + "/reports/")) for name in bundle.namelist())
        for name in bundle.namelist():
            if not (destination / name).resolve().is_relative_to(destination.resolve()):
                raise RuntimeError("Unsafe ZIP path")
        bundle.extractall(destination)
    with zipfile.ZipFile(ROOT / "dist" / f"FastASR-{VERSION}-source.zip") as bundle:
        checks["source_crc"] = bundle.testzip() is None
        required = ["LICENSE", "Cargo.lock", "src/xasr_bridge.cpp", "scripts/prepare_assets.py", "scripts/verify.ps1", "scripts/prepare_xasr.py", "scripts/asset_download.py", ".github/workflows/windows.yml", "vendor/samples/asr/46.wav"]
        required += ["scripts/check_licenses.py", "vendor/licenses/SOURCES.json", "docs/LEGAL.md", "docs/sample-provenance.md"]
        checks["source_notices_and_disclosures"] = all(
            source.relative_to(ROOT).as_posix() in bundle.namelist()
            and hashlib.sha256(bundle.read(source.relative_to(ROOT).as_posix())).hexdigest() == digest(source)
            for source, _ in notices
        )
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
