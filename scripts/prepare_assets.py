"""Prepare both supported model engines; retired files are left untouched."""
from check_licenses import license_materials
from prepare_xasr import main as xasr
from prepare_whisper import main as whisper
if __name__=="__main__":
 license_materials()
 xasr()
 whisper()
