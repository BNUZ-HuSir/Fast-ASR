"""Prepare the X-ASR model and CPU runtime used by this release."""
from check_licenses import license_materials
from prepare_xasr import main as xasr

if __name__ == "__main__":
    license_materials()
    xasr()
