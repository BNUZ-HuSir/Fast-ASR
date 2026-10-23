"""Shared version and source inventory for reviewable release archives."""
from pathlib import Path
import hashlib
import re

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", (ROOT / "Cargo.toml").read_text(encoding="utf-8")).group(1)
VERSION = re.search(r'(?m)^version\s*=\s*"([^"]+)"', PACKAGE).group(1)
NAME = f"FastASR-{VERSION}-win-x64"
REPORT_NAMES = ["xasr-self-test.json", "turbo-self-test.json", "pipeline-probe.json"]
UI_REPORT_NAMES = ["input-probe.json", "ui-probe.json"]

def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def source_files():
    files = [ROOT / n for n in ["Cargo.toml", "Cargo.lock", "build.rs", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md", "CHANGELOG.md", "CONTRIBUTING.md", "SECURITY.md", ".gitignore"]]
    files += sorted((ROOT / "src").glob("*.rs"))
    files += [ROOT / "src" / n for n in ["whisper_bridge.cpp", "xasr_bridge.cpp"]]
    files += [ROOT / "assets" / n for n in ["app.ico", "app.rc", "app.manifest", "whisper-manifest.json", "xasr-manifest.json"]]
    for folder, pattern in [(".cargo", "*.toml"), (".github", "*"), ("scripts", "*.py"), ("scripts", "*.ps1"), ("docs", "*.md"), ("docs/images", "*.png"), ("vendor/licenses", "*"), ("vendor/samples/asr", "*.wav")]:
        files += [p for p in sorted((ROOT / folder).rglob(pattern)) if p.is_file() and "__pycache__" not in p.parts and p.name != "build_xasr_trial.ps1"]
    return sorted(set(files))
