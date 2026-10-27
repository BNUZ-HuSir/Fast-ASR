"""Prepare pinned bilingual X-ASR 480 ms model and native CPU runtime; no deletion."""
from pathlib import Path
import json, shutil, tarfile
from asset_download import fetch, sha
ROOT=Path(__file__).resolve().parents[1]
def extract(archive, destination):
 destination.mkdir(parents=True,exist_ok=True)
 with tarfile.open(archive,'r:bz2') as tar:
  for member in tar.getmembers():
   path=(destination/member.name).resolve()
   if not path.is_relative_to(destination.resolve()) or not (member.isfile() or member.isdir()):
    raise RuntimeError('Unsafe archive entry: '+member.name)
   if member.isdir():path.mkdir(parents=True,exist_ok=True)
   else:
    path.parent.mkdir(parents=True,exist_ok=True)
    with tar.extractfile(member) as src, path.open('wb') as dst:shutil.copyfileobj(src,dst)
def main():
 manifest=json.loads((ROOT/'assets/xasr-manifest.json').read_text(encoding='utf-8'))
 archive=ROOT/'vendor/downloads/x-asr-480ms-punct-int8-research.tar.bz2'
 fetch(manifest['source'],archive,manifest['archive_bytes'],manifest['archive_sha256'])
 model=ROOT/'vendor/models'/manifest['directory']
 if not all((model/f['name']).is_file() and sha(model/f['name'])==f['sha256'] for f in manifest['files']):extract(archive,ROOT/'vendor/models')
 for f in manifest['files']:
  if (model/f['name']).stat().st_size!=f['bytes'] or sha(model/f['name'])!=f['sha256']:raise RuntimeError('Model file verification failed: '+f['name'])
 runtime=manifest['runtime'];archive=ROOT/'vendor/downloads'/(runtime['directory']+'.tar.bz2')
 fetch(runtime['source'],archive,runtime['bytes'],runtime['sha256'])
 destination=ROOT/'vendor/runtime'/runtime['directory']
 if not all((destination/p).is_file() for p in ['include/sherpa-onnx/c-api/c-api.h','lib/sherpa-onnx-c-api.dll','lib/onnxruntime.dll','lib/onnxruntime_providers_shared.dll']):extract(archive,ROOT/'vendor/runtime')
 print('X-ASR 480 ms resources verified',flush=True)
if __name__=='__main__':main()
