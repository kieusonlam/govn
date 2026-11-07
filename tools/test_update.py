"""The "check for updates" menu item against the real GitHub API, reading the dialog it shows.

Keys are not touched (--only-pid 0) and the settings file and registry key are temporary. It answers
No, so no browser opens. Quit govn first: two copies cannot run together.

usage: test_update.py <govn.exe> <expected text in the dialog>
"""
import ctypes
import pathlib
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
exe, expected = sys.argv[1:3]

WM_COMMAND, WM_CLOSE, IDNO = 0x111, 0x10, 7
CMD_CHECK_UPDATE, CMD_QUIT = 1050, 1099

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.FindWindowW.restype = wintypes.HWND
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetDlgItem.restype = wintypes.HWND
user32.GetDlgItem.argtypes = [wintypes.HWND, ctypes.c_int]
EnumProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumChildWindows.argtypes = [wintypes.HWND, EnumProc, wintypes.LPARAM]


def wait_for(check, secs):
    end = time.time() + secs
    while time.time() < end:
        found = check()
        if found:
            return found
        time.sleep(0.1)
    return None


def dialog_text(dialog):
    texts = []

    def collect(child, _):
        buf = ctypes.create_unicode_buffer(512)
        user32.GetWindowTextW(child, buf, 512)
        texts.append(buf.value)
        return True

    user32.EnumChildWindows(dialog, EnumProc(collect), 0)
    return "\n".join(texts)


if "govn.exe" in subprocess.run(["tasklist", "/fi", "imagename eq govn.exe", "/fo", "csv", "/nh"], capture_output=True, text=True).stdout:
    raise SystemExit("govn đang chạy: hãy thoát nó trước. Chưa làm gì cả.")

tmp = pathlib.Path(tempfile.mkdtemp(prefix="govn-update-"))
proc = subprocess.Popen([exe, "--only-pid", "0", "--duration-secs", "40", "--config", str(tmp / "settings.ini"),
                         "--task-name", "govn-e2e-test", "--autostart-key", r"Software\govn-e2e-test\Run"])
ok = False
try:
    window = wait_for(lambda: user32.FindWindowW("GovnTrayWindow", None), 8)
    if not window:
        raise SystemExit("govn không mở được")
    user32.PostMessageW(window, WM_COMMAND, CMD_CHECK_UPDATE, 0)
    dialog = wait_for(lambda: user32.FindWindowW("#32770", "govn"), 25)
    if not dialog:
        raise SystemExit("không thấy hộp thoại kết quả")
    text = dialog_text(dialog)
    print(text)
    ok = expected in text
    print("ok" if ok else f"FAIL: chờ {expected!r}")
    # No only exists on the "update available" dialog; a plain OK closes with WM_CLOSE.
    if user32.GetDlgItem(dialog, IDNO):
        user32.PostMessageW(dialog, WM_COMMAND, IDNO, 0)
    else:
        user32.PostMessageW(dialog, WM_CLOSE, 0, 0)
    time.sleep(0.5)
    user32.PostMessageW(window, WM_COMMAND, CMD_QUIT, 0)
    proc.wait(timeout=8)
finally:
    if proc.poll() is None:
        proc.kill()
    subprocess.run(["reg", "delete", r"HKCU\Software\govn-e2e-test", "/f"], capture_output=True)
    import shutil
    shutil.rmtree(tmp, ignore_errors=True)
sys.exit(0 if ok else 1)
