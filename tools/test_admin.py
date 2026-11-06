"""Real test with an elevated window. Runs with normal rights and starts `e2e --admin` elevated,
so Windows shows one UAC prompt. Defender has removed the files and the task when this ran from the
temporary build folder, so a blocked run is not a keyboard failure; see README.

It only touches a scheduled task named govn-e2e-test and the key HKCU\\Software\\govn-e2e-test, and
removes both.

usage: test_admin.py <govn.exe> <e2e.exe>
"""
import ctypes
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
govn, e2e = sys.argv[1:3]

TASK = "govn-e2e-test"
RUN_KEY = r"Software\govn-e2e-test\Run"
WM_COMMAND, WM_APP, WM_LBUTTONUP = 0x111, 0x8000, 0x202
WM_TRAY = WM_APP + 1
CMD_AUTOSTART, CMD_ADMIN, CMD_QUIT = 1040, 1041, 1099

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.FindWindowW.restype = wintypes.HWND
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
shell32 = ctypes.WinDLL("shell32")
shell32.ShellExecuteW.restype = ctypes.c_ssize_t
shell32.ShellExecuteW.argtypes = [wintypes.HWND] + [wintypes.LPCWSTR] * 4 + [ctypes.c_int]

tmp = pathlib.Path(tempfile.mkdtemp(prefix="govn-admin-"))
failed = total = 0


def report(name, passed, detail=""):
    global failed, total
    total += 1
    failed += not passed
    print(("ok    " if passed else "FAIL  ") + name + (f": {detail}" if detail else ""), flush=True)


def tray():
    return user32.FindWindowW("GovnTrayWindow", None)


def signal(name):
    (tmp / name).write_text("")


def wait(name, secs=120):
    end = time.time() + secs
    while time.time() < end:
        if (tmp / "abort").exists():
            raise SystemExit("phía admin dừng sớm")
        if (tmp / name).exists():
            return
        time.sleep(0.05)
    raise SystemExit(f"hết giờ chờ {name}")


def query_xml():
    r = subprocess.run(["schtasks", "/query", "/tn", TASK, "/xml"], capture_output=True)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def run_value():
    return subprocess.run(["reg", "query", "HKCU\\" + RUN_KEY, "/v", "govn"], capture_output=True).returncode == 0


def wait_gone(proc, secs):
    try:
        proc.wait(timeout=secs)
        return True
    except subprocess.TimeoutExpired:
        return False


def own_processes():
    out = subprocess.run(["tasklist", "/fi", "imagename eq govn.exe", "/fo", "csv", "/nh"], capture_output=True, text=True).stdout
    return "govn.exe" in out


if own_processes():
    raise SystemExit("govn đang chạy: hãy thoát nó trước, hai bản không chạy cùng lúc được. Chưa làm gì cả.")

# Files an elevated process creates here are unreadable for this one, so it writes into these instead.
(tmp / "pid_value").write_text("")
(tmp / "results.txt").write_text("", encoding="utf-8")

try:
    rc = shell32.ShellExecuteW(None, "runas", e2e, f'"{govn}" --admin "{tmp}"', None, 1)
    if rc <= 32:
        raise SystemExit(f"không chạy được bản admin (UAC bị từ chối?), mã {rc}")
    wait("pid", 180)
    pid = (tmp / "pid_value").read_text()

    # 1. Normal keyboard, elevated window.
    kb = subprocess.Popen([govn, "--only-pid", pid, "--accept-injected", "--duration-secs", "30"])
    time.sleep(0.6)
    signal("normal_started")
    wait("normal_checked")
    kb.kill()
    kb.wait()
    time.sleep(0.3)
    signal("normal_stopped")

    # 2. Elevated keyboard running: a normal one must refuse to start.
    wait("elevated_running")
    second = subprocess.run([govn, "--only-pid", pid, "--accept-injected", "--duration-secs", "3"], timeout=15)
    report("bản thường từ chối chạy khi bản admin đang chạy", second.returncode == 1, f"mã thoát {second.returncode}")
    signal("second_checked")
    wait("second_verified")

    # 3. The shell runs with normal rights, like this script.
    ok = user32.PostMessageW(tray(), WM_TRAY, 0, WM_LBUTTONUP)
    report("cửa sổ khay bản admin nhận thông điệp từ quyền thường", bool(ok))
    time.sleep(0.3)
    ok = user32.PostMessageW(tray(), WM_COMMAND, CMD_QUIT, 0)
    report("lệnh từ quyền thường vẫn bị Windows chặn (chứng tỏ đúng là bản admin)", not ok)
    signal("tray_clicked")

    # 4. The task, as Windows stored it.
    wait("task_created")
    xml = query_xml()
    report("task tồn tại", xml is not None)
    if xml:
        for needed in ("<RunLevel>HighestAvailable</RunLevel>",
                       "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
                       "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
                       "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
                       "<Priority>4</Priority>"):
            report(f"task có {needed}", needed in xml)

    run = subprocess.run(["schtasks", "/run", "/tn", TASK], capture_output=True)
    report("quyền thường chạy được task, không hỏi UAC", run.returncode == 0)
    time.sleep(1.5)
    report("task đã mở govn", own_processes())
    time.sleep(6)
    report("govn của task tự thoát", not own_processes())

    signal("delete_first")
    wait("first_deleted")
    report("task đã xóa trước khi thử menu", query_xml() is None)

finally:
    signal("delete_first")
    signal("delete_task")
    try:
        wait("task_deleted", 20)
    except SystemExit as e:
        print(e)
    report("task test đã được dọn", query_xml() is None)
    log = tmp / "results.txt"
    print("--- phía admin")
    if log.exists():
        text = log.read_text(encoding="utf-8")
        print(text, end="")
        n_fail = text.count("FAIL  ")
        failed += n_fail
        total += text.count("ok    ") + n_fail
    time.sleep(0.5)
    shutil.rmtree(tmp, ignore_errors=True)

print(f"đạt {total - failed}/{total}")
sys.exit(1 if failed else 0)
