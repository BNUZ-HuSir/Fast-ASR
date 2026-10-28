"""Check real hook teardown on an inactive private desktop, without injected input."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import time
import uuid
from release_common import ROOT, VERSION, digest

class STARTUPINFOW(C.Structure):
    _fields_ = [("cb", W.DWORD), ("lpReserved", W.LPWSTR), ("lpDesktop", W.LPWSTR),
                ("lpTitle", W.LPWSTR), ("dwX", W.DWORD), ("dwY", W.DWORD),
                ("dwXSize", W.DWORD), ("dwYSize", W.DWORD), ("dwXCountChars", W.DWORD),
                ("dwYCountChars", W.DWORD), ("dwFillAttribute", W.DWORD),
                ("dwFlags", W.DWORD), ("wShowWindow", W.WORD), ("cbReserved2", W.WORD),
                ("lpReserved2", C.POINTER(W.BYTE)), ("hStdInput", W.HANDLE),
                ("hStdOutput", W.HANDLE), ("hStdError", W.HANDLE)]

class PROCESS_INFORMATION(C.Structure):
    _fields_ = [("hProcess", W.HANDLE), ("hThread", W.HANDLE),
                ("dwProcessId", W.DWORD), ("dwThreadId", W.DWORD)]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=ROOT / ".build/target/release/fast-asr.exe")
    parser.add_argument("--home", type=Path, default=ROOT)
    args = parser.parse_args()
    executable, home = args.exe.resolve(), args.home.resolve()
    if not home.is_relative_to(ROOT) or not executable.is_relative_to(ROOT):
        raise RuntimeError("Checks must stay inside this workspace")
    reports = home / "reports"
    reports.mkdir(parents=True, exist_ok=True)
    user = C.WinDLL("user32", use_last_error=True)
    kernel = C.WinDLL("kernel32", use_last_error=True)
    user.CreateDesktopW.argtypes = [W.LPCWSTR, W.LPCWSTR, C.c_void_p, W.DWORD, W.DWORD, C.c_void_p]
    user.CreateDesktopW.restype = W.HANDLE
    user.CloseDesktop.argtypes = [W.HANDLE]
    kernel.CreateProcessW.argtypes = [W.LPCWSTR, W.LPWSTR, C.c_void_p, C.c_void_p, W.BOOL,
                                    W.DWORD, C.c_void_p, W.LPCWSTR,
                                    C.POINTER(STARTUPINFOW), C.POINTER(PROCESS_INFORMATION)]
    kernel.CreateProcessW.restype = W.BOOL
    kernel.WaitForSingleObject.argtypes = [W.HANDLE, W.DWORD]
    kernel.WaitForSingleObject.restype = W.DWORD
    kernel.GetExitCodeProcess.argtypes = [W.HANDLE, C.POINTER(W.DWORD)]
    kernel.CloseHandle.argtypes = [W.HANDLE]
    desktop_name = "FastASR-check-" + uuid.uuid4().hex
    desktop = user.CreateDesktopW(desktop_name, None, None, 0, 0x10000000, None)
    if not desktop:
        raise C.WinError(C.get_last_error())
    process = PROCESS_INFORMATION()
    result = {"version": VERSION, "executable_sha256": digest(executable), "passed": False,
              "private_desktop": True, "desktop_switched": False,
              "input_injected": False, "microphone_opened": False}
    started = time.monotonic()
    try:
        startup = STARTUPINFOW()
        startup.cb = C.sizeof(startup)
        startup.lpDesktop = desktop_name
        os.environ["FAST_ASR_HOME"] = str(home)
        command = C.create_unicode_buffer('"' + str(executable) + '" --hook-lifecycle-probe')
        if not kernel.CreateProcessW(str(executable), command, None, None, False,
                                     0x08000000, None, str(home), C.byref(startup), C.byref(process)):
            raise C.WinError(C.get_last_error())
        result["process_id"] = process.dwProcessId
        wait = kernel.WaitForSingleObject(process.hProcess, 10000)
        result["process_wait_result"] = wait
        result["process_exit_signaled"] = wait == 0
        if wait != 0:
            raise RuntimeError("Lifecycle probe has not completed process exit; no further desktop probes should run")
        code = W.DWORD()
        if not kernel.GetExitCodeProcess(process.hProcess, C.byref(code)):
            raise C.WinError(C.get_last_error())
        result["exit_code"] = code.value
        details = json.loads((reports / "hook-lifecycle-probe.json").read_text(encoding="utf-8"))
        result["probe"] = details
        result["passed"] = (code.value == 0 and details.get("passed") is True
                            and details.get("process_id") == process.dwProcessId
                            and details.get("executable_sha256") == result["executable_sha256"]
                            and details.get("version") == VERSION and details.get("cycles") == 5)
    except Exception as error:
        result["error"] = str(error)
    finally:
        for handle in [process.hThread, process.hProcess]:
            if handle:
                kernel.CloseHandle(handle)
        user.CloseDesktop(desktop)
        result["elapsed_seconds"] = time.monotonic() - started
        (reports / "hook-lifecycle.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)
    if not result["passed"]:
        raise SystemExit(1)

if __name__ == "__main__":
    main()
