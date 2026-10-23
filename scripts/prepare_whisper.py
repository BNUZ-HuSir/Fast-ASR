"""Prepare pinned Whisper + Silero weights and build an AVX2 Windows CPU runtime locally."""
from pathlib import Path
import concurrent.futures,hashlib,json,urllib.request,zipfile,subprocess,shutil,time,os
ROOT=Path(__file__).resolve().parents[1]
(ROOT/'.build').mkdir(parents=True,exist_ok=True)
os.environ['TEMP']=os.environ['TMP']=str(ROOT/'.build')
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
def fetch(url,p,size,digest):
 p.parent.mkdir(parents=True,exist_ok=True)
 if p.exists() and p.stat().st_size==size and sha(p)==digest:return
 part=p.with_suffix('.download')
 if part.exists() and part.stat().st_size>=size:
  if part.stat().st_size==size and sha(part)==digest:
   if p.exists():p.rename(p.with_suffix('.damaged-'+str(time.time_ns())))
   part.replace(p);return
  part.rename(part.with_suffix('.damaged-'+str(time.time_ns())))
 failures=0
 while (part.stat().st_size if part.exists() else 0)<size:
  offset=part.stat().st_size if part.exists() else 0
  try:
   end=min(offset+4*1024*1024,size)-1
   request=urllib.request.Request(url+('&' if '?' in url else '?')+'download=true&part='+str(offset),headers={'Range':f'bytes={offset}-{end}','User-Agent':'FastASR/0.3.1'})
   with urllib.request.urlopen(request,timeout=45) as r:
    if r.status==206 and not r.headers.get('Content-Range','').startswith(f'bytes {offset}-'):raise RuntimeError('Mismatched Content-Range')
    if offset and r.status!=206:raise RuntimeError('Server cannot resume')
    remaining=(end-offset+1) if r.status==206 else size
    with part.open('ab') as f:
     while remaining:
      b=r.read(min(65536,remaining))
      if not b:break
      f.write(b);remaining-=len(b)
   print(p.name,part.stat().st_size,'/',size,flush=True)
  except Exception as e:print('Retry:',p.name,str(e)[:120],flush=True)
  progress=part.stat().st_size if part.exists() else 0
  failures=0 if progress>offset else failures+1
  if failures>=6:raise RuntimeError('Download made no progress: '+str(p))
 if not part.exists() or part.stat().st_size!=size or sha(part)!=digest:
  if part.exists():part.rename(part.with_suffix('.damaged-'+str(time.time_ns())))
  raise RuntimeError('Integrity check failed; preserved invalid download: '+str(p))
 if p.exists():p.rename(p.with_suffix('.damaged-'+str(time.time_ns())))
 part.replace(p)
def main():
 m=json.loads((ROOT/'assets/whisper-manifest.json').read_text(encoding='utf-8'))
 jobs=[(m['repository']+'/resolve/'+m['revision']+'/'+f['name'],ROOT/'vendor/models/whisper'/f['name'],f['bytes'],f['sha256']) for f in m['files']]
 jobs.extend([
 ('https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v5.1.2.bin',ROOT/'vendor/models/vad/ggml-silero-v5.1.2.bin',885098,'29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf'),
 ('https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v1.9.5.zip',ROOT/'vendor/downloads/whisper-source-v1.9.5.zip',10640105,'7a7a73071939f16bc028ad2f713f16f26e37afb6cc155d0f19e3d65705d2b1d9')])
 with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(lambda j:fetch(*j),jobs))
 target=ROOT/'vendor/whisper-source';target.mkdir(parents=True,exist_ok=True)
 if not (target/'whisper.cpp-1.9.5/CMakeLists.txt').exists():
  with zipfile.ZipFile(ROOT/'vendor/downloads/whisper-source-v1.9.5.zip') as z:
   for n in z.namelist():
    if not (target/n).resolve().is_relative_to(target.resolve()):raise RuntimeError('Unsafe ZIP')
   z.extractall(target)
 build=ROOT/'.build/whisper-x64'
 subprocess.run(['cmake','-S',str(target/'whisper.cpp-1.9.5'),'-B',str(build),'-A','x64','-DBUILD_SHARED_LIBS=ON','-DWHISPER_BUILD_TESTS=OFF','-DWHISPER_BUILD_EXAMPLES=OFF','-DGGML_NATIVE=OFF','-DGGML_OPENMP=OFF','-DGGML_BACKEND_DL=OFF','-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded'],check=True,cwd=ROOT)
 subprocess.run(['cmake','--build',str(build),'--config','Release','--target','whisper','-j','4'],check=True,cwd=ROOT)
 runtime=ROOT/'vendor/runtime/whisper-1.9.5-win-x64-cpu';runtime.mkdir(parents=True,exist_ok=True)
 for name in ['whisper.dll','ggml.dll','ggml-base.dll','ggml-cpu.dll']:shutil.copy2(build/'bin/Release'/name,runtime/name)
 print('Whisper resources ready')
if __name__=='__main__':main()
