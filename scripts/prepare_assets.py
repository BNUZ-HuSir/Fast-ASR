"""Prepare both supported model engines; retired files are left untouched."""
from prepare_xasr import main as xasr
from prepare_whisper import main as whisper
if __name__=="__main__":
 xasr()
 whisper()
