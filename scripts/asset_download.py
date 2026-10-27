"""Shared pinned-download helpers; keeps incomplete and damaged files for recovery."""
from pathlib import Path
import hashlib
import time
import urllib.request

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
   request=urllib.request.Request(url+('&' if '?' in url else '?')+'download=true&part='+str(offset),headers={'Range':f'bytes={offset}-{end}','User-Agent':'FastASR/0.3.5'})
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
