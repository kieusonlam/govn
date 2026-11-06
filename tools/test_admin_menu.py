"""Turning admin mode on from the menu, with govn.exe in Program Files, as a user would.

Shows two UAC prompts: one for the helper that copies the file and later cleans up, one from govn's
own menu command. Keys are not touched (--only-pid 0). The test uses a scheduled task named
govn-e2e-test, the key HKCU\\Software\\govn-e2e-test and the folders below, and removes all of them
through the elevated helper, which does not depend on govn.exe surviving the test.

usage: test_admin_menu.py <govn.exe>
"""
import ctypes
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
source = sys.argv[1]

TASK = "govn-e2e-test"
REG = r"Software\govn-e2e-test"
RUN_KEY = REG + r"\Run"
INSTALL = pathlib.Path(os.environ["ProgramFiles"]) / "govn"
EXE = str(INSTALL / "govn.exe")
SETTINGS_DIR = pathlib.Path(os.environ["APPDATA"]) / "govn-e2e-test"
CFG = SETTINGS_DIR / "settings.ini"
WM_COMMAND, WM_TRAY, WM_LBUTTONUP = 0x111, 0x8001, 0x202
CMD_AUTOSTART, CMD_ADMIN, CMD_QUIT = 1040, 1041, 1099

HELPER = r"""#Requires -Version 7
param([string]$Dir, [string]$Source, [string]$Install, [string]$Task)
try {
    New-Item -ItemType Directory -Force $Install | Out-Null
    Copy-Item -LiteralPath $Source -Destination (Join-Path $Install 'govn.exe') -Force
    New-Item (Join-Path $Dir 'copied') | Out-Null
    $end = (Get-Date).AddMinutes(6)
    while ((Get-Date) -lt $end -and -not (Test-Path (Join-Path $Dir 'cleanup'))) { Start-Sleep -Milliseconds 200 }
} finally {
    & (Join-Path $env:SystemRoot 'System32\schtasks.exe') /delete /tn $Task /f 2>&1 | Out-Null
    Get-Process govn -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Install*" } | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Remove-Item -LiteralPath $Install -Recurse -Force -ErrorAction SilentlyContinue
    New-Item (Join-Path $Dir 'cleaned') | Out-Null
}
"""

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.FindWindowW.restype = wintypes.HWND
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
shell32 = ctypes.WinDLL("shell32")
shell32.ShellExecuteW.restype = ctypes.c_ssize_t
shell32.ShellExecuteW.argtypes = [wintypes.HWND] + [wintypes.LPCWSTR] * 4 + [ctypes.c_int]

failed = total = 0


def report(name, passed, detail=""):
    global failed, total
    total += 1
    failed += not passed
    print(("ok    " if passed else "FAIL  ") + name + (f": {detail}" if detail else ""), flush=True)


def tray():
    return user32.FindWindowW("GovnTrayWindow", None)


def wait_for(check, secs):
    end = time.time() + secs
    while time.time() < end:
        if check():
            return True
        time.sleep(0.1)
    return False


def task_xml():
    r = subprocess.run(["schtasks", "/query", "/tn", TASK, "/xml"], capture_output=True)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def run_value():
    return subprocess.run(["reg", "query", "HKCU\\" + RUN_KEY, "/v", "govn"], capture_output=True).returncode == 0


def govn_running():
    out = subprocess.run(["tasklist", "/fi", "imagename eq govn.exe", "/fo", "csv", "/nh"], capture_output=True, text=True).stdout
    return "govn.exe" in out


if govn_running():
    raise SystemExit("govn đang chạy: hãy thoát nó trước. Chưa làm gì cả.")
if INSTALL.exists():
    raise SystemExit(f"{INSTALL} đã có sẵn: không đụng vào. Chưa làm gì cả.")

started = time.strftime("%H:%M:%S")
tmp = pathlib.Path(tempfile.mkdtemp(prefix="govn-menu-"))
helper = tmp / "helper.ps1"
helper.write_text(HELPER, encoding="utf-8")
args = f'-NoProfile -File "{helper}" -Dir "{tmp}" -Source "{source}" -Install "{INSTALL}" -Task {TASK}'
if shell32.ShellExecuteW(None, "runas", shutil.which("pwsh"), args, None, 0) <= 32:
    shutil.rmtree(tmp, ignore_errors=True)
    raise SystemExit("không chạy được tiến trình dọn dẹp (UAC bị từ chối?). Chưa làm gì cả.")

common = ["--config", str(CFG), "--autostart-key", RUN_KEY, "--task-name", TASK]
task_args = f'--only-pid 0 --duration-secs 12 --config "{CFG}" --autostart-key {RUN_KEY} --task-name {TASK}'
try:
    if not wait_for(lambda: (tmp / "copied").exists(), 120):
        raise SystemExit("hết giờ chờ chép file (chưa bấm UAC?)")
    report("đã chép vào Program Files", os.path.exists(EXE))

    kb = subprocess.Popen([EXE, "--only-pid", "0", "--duration-secs", "200", *common, "--task-args", task_args])
    if not wait_for(lambda: tray() is not None, 8):
        raise SystemExit("bản quyền thường không mở được")
    user32.PostMessageW(tray(), WM_COMMAND, CMD_AUTOSTART, 0)
    time.sleep(0.4)
    report("có mục Run trước khi bật", run_value())

    user32.PostMessageW(tray(), WM_COMMAND, CMD_ADMIN, 0)
    print("      (chờ bấm Yes ở UAC thứ hai)", flush=True)
    try:
        kb.wait(timeout=120)
        report("bản quyền thường tự thoát sau khi bật", True)
    except subprocess.TimeoutExpired:
        report("bản quyền thường tự thoát sau khi bật", False)
        raise SystemExit("dừng: chưa bật được")

    xml = task_xml()
    report("task đã tạo", xml is not None)
    if xml:
        report("task trỏ tới Program Files", EXE.lower() in xml.lower())
        report("task chạy quyền cao nhất", "<RunLevel>HighestAvailable</RunLevel>" in xml)
    report("đã gỡ mục Run để không chạy hai lần", not run_value())

    report("task đã mở lại govn", wait_for(lambda: tray() is not None, 8))
    time.sleep(0.5)
    report("bản mới đúng là quyền admin (Windows chặn lệnh từ quyền thường)", not user32.PostMessageW(tray(), WM_COMMAND, CMD_QUIT, 0))

    before = CFG.read_text(encoding="utf-8") if CFG.exists() else ""
    report("nhấp icon vẫn gửi được tới bản admin", bool(user32.PostMessageW(tray(), WM_TRAY, 0, WM_LBUTTONUP)))
    time.sleep(0.6)
    try:
        after = CFG.read_text(encoding="utf-8")
        report("bản admin lưu cài đặt, quyền thường vẫn đọc được", "viet=0" in after and after != before)
    except OSError as e:
        report("bản admin lưu cài đặt, quyền thường vẫn đọc được", False, str(e))
    user32.PostMessageW(tray(), WM_TRAY, 0, WM_LBUTTONUP)
    time.sleep(0.4)

    report("bản admin tự thoát hết giờ", wait_for(lambda: not govn_running(), 16))

    # Opened by hand while admin mode is on: must hand over rather than run with normal rights.
    direct = subprocess.run([EXE, *common], timeout=20)
    report("mở tay khi đã bật: chuyển sang task rồi thoát", direct.returncode == 0, f"mã thoát {direct.returncode}")
    report("mở tay: bản admin chạy lên", wait_for(lambda: tray() is not None, 8))
    time.sleep(0.5)
    report("mở tay: bản đó quyền admin", not user32.PostMessageW(tray(), WM_COMMAND, CMD_QUIT, 0))
except OSError as e:
    report("chạy được govn.exe", False, str(e))
finally:
    (tmp / "cleanup").write_text("")
    cleaned = wait_for(lambda: (tmp / "cleaned").exists(), 30)
    subprocess.run(["reg", "delete", "HKCU\\" + REG, "/f"], capture_output=True)
    report("dọn: tiến trình dọn dẹp xong", cleaned)
    report("dọn: không còn task", task_xml() is None)
    report("dọn: không còn thư mục Program Files", not INSTALL.exists())
    report("dọn: không còn govn chạy", not govn_running())
    shutil.rmtree(SETTINGS_DIR, ignore_errors=True)
    shutil.rmtree(tmp, ignore_errors=True)

    print("--- Defender từ " + started)
    ps = ("Get-WinEvent -LogName 'Microsoft-Windows-Windows Defender/Operational' -MaxEvents 60 | "
          "Where-Object { $_.Id -in 1116,1117 -and $_.TimeCreated -ge (Get-Date '" + started + "') } | "
          "ForEach-Object { '{0:HH:mm:ss} id={1} {2}' -f $_.TimeCreated, $_.Id, "
          "((($_.Message -split [char]10) | Where-Object { $_ -match '^\\s*(Name|Path):' } | "
          "ForEach-Object { $_.Trim() }) -join ' | ') }")
    out = subprocess.run(["pwsh", "-NoProfile", "-Command", ps], capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    print(out.strip() or "không có phát hiện mới")

print(f"đạt {total - failed}/{total}")
sys.exit(1 if failed else 0)
