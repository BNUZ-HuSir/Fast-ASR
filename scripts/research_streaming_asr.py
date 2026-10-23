"""Reproduce the X-ASR candidate CPU smoke comparison; not an accuracy benchmark."""
import ctypes
from ctypes import wintypes
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import tarfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
MODEL_ROOT = ROOT / 'vendor/models'
RUNTIME = ROOT / 'vendor/runtime/sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts/bin/sherpa-onnx.exe'
ARCHIVES = {
    480: ('x-asr-480ms-punct-int8-research.tar.bz2', 'fa5f63d618e5a01526e275a358bb7772e403f84808a4769fba52cffd8160bf74'),
    960: ('x-asr-960ms-punct-int8-research.tar.bz2', '0a92b798bd6801c333c7ce8aebf5ba769bfe7f3f3511699a67837b2288428603'),
}

class MemoryCounters(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            'PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
            'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
            'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]

get_memory = ctypes.WinDLL('psapi').GetProcessMemoryInfo
get_memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
get_memory.restype = wintypes.BOOL


def unpack(delay):
    name, expected_hash = ARCHIVES[delay]
    archive = ROOT / 'vendor/downloads' / name
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != expected_hash:
        raise RuntimeError('Archive hash mismatch: ' + str(archive))
    model = MODEL_ROOT / ('sherpa-onnx-x-asr-%dms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05' % delay)
    if not model.exists():
        with tarfile.open(archive, 'r:bz2') as tar:
            members = tar.getmembers()
            for member in members:
                target = (MODEL_ROOT / member.name).resolve()
                if MODEL_ROOT.resolve() not in target.parents:
                    raise RuntimeError('Unsafe archive path: ' + member.name)
                if not (member.isdir() or member.isfile()):
                    raise RuntimeError('Unsupported archive entry: ' + member.name)
            for member in members:
                target = MODEL_ROOT / member.name
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(member) as source, target.open('wb') as output:
                        shutil.copyfileobj(source, output)
    return model, digest


def run(delay, method):
    model, archive_hash = unpack(delay)
    name = 'x-asr-%dms-int8-%s-padded' % (delay, method)
    samples = sorted((ROOT / 'vendor/samples/asr').glob('*.wav'))
    arguments = [str(RUNTIME), '--tokens=' + str(model/'tokens.txt'),
                 '--encoder=' + str(model/'encoder.int8.onnx'),
                 '--decoder=' + str(model/'decoder.onnx'),
                 '--joiner=' + str(model/'joiner.int8.onnx'),
                 '--provider=cpu', '--num-threads=4', '--right-padding=2.0', '--decoding-method=' + method]
    if method == 'modified_beam_search':
        arguments.append('--max-active-paths=4')
    arguments.extend(str(path) for path in samples)
    stdout_path = ROOT / 'reports' / (name + '.stdout.txt')
    stderr_path = ROOT / 'reports' / (name + '.stderr.txt')
    started = time.perf_counter()
    peak_working = peak_private = 0
    with stdout_path.open('wb') as out, stderr_path.open('wb') as err:
        process = subprocess.Popen(arguments, stdout=out, stderr=err, creationflags=subprocess.CREATE_NO_WINDOW)
        while process.poll() is None:
            counters = MemoryCounters()
            counters.cb = ctypes.sizeof(counters)
            if get_memory(int(process._handle), ctypes.byref(counters), counters.cb):
                peak_working = max(peak_working, counters.PeakWorkingSetSize)
                peak_private = max(peak_private, counters.PeakPagefileUsage)
            time.sleep(0.02)
    elapsed = time.perf_counter() - started
    stdout = stdout_path.read_text(encoding='utf-8', errors='replace')
    stderr = stderr_path.read_text(encoding='utf-8', errors='replace')
    transcripts = []
    for line in stderr.splitlines():
        if line.startswith('{ "text":'):
            transcripts.append(json.loads(line))
    times = re.findall(r'Elapsed seconds: ([\d.]+), Audio duration \(s\): ([\d.]+), Real time factor \(RTF\) = [^\n]+? = ([\d.]+)', stderr)
    load = re.search(r'Recognizer created in ([\d.]+) s', stdout)
    weights = {path.name: path.stat().st_size for path in sorted(model.glob('*.onnx'))}
    report = dict(model='X-ASR zh-en punct INT8', date='2026-10-22', chunk_ms=delay,
                  decoding=method, provider='cpu', threads=4, right_padding_seconds=2.0, exit_code=process.returncode,
                  model_weight_bytes=sum(weights.values()), weight_files=weights,
                  archive_sha256=archive_hash, wall_seconds=elapsed,
                  load_seconds=float(load.group(1)) if load else None,
                  peak_working_set_mib=peak_working/1048576,
                  peak_private_commit_mib=peak_private/1048576,
                  arguments=arguments, cases=[dict(sample=str(p.relative_to(ROOT)), text=t['text'],
                    processing_seconds=float(timing[0]), audio_seconds=float(timing[1]), rtf=float(timing[2]))
                    for p,t,timing in zip(samples,transcripts,times)],
                  limitations=['CLI runs multiple streams in parallel; its per-stream RTF includes concurrency.',
                               'WAV smoke test, not a real microphone latency measurement.',
                               'No independently verified reference transcripts; no CER or WER claim.',
                               'Process memory excludes the desktop GUI and does not measure GPU memory.'])
    if process.returncode or len(transcripts) != len(samples) or len(times) != len(samples):
        raise RuntimeError('Incomplete test: %s; see %s' % (name, stderr_path))
    report_path = ROOT / 'reports' / (name + '.json')
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False), flush=True)
    return report

if __name__ == '__main__':
    for delay, method in [(480, 'greedy_search'), (960, 'greedy_search'), (960, 'modified_beam_search')]:
        run(delay, method)
