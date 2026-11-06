"""Real VS Code test, with a throwaway profile and no extensions. Text is read back from the auto-saved file.

usage: test_vscode.py <Code.exe> <govn.exe> <sendkeys.exe> <auto|always|never>
"""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
code, govn, sendkeys, fix = sys.argv[1:5]

CASES = [
    ("gox", "gõ"),
    ("gos", "gó"),
    ("gooj", "gộ"),
    ("goxx", "gox"),
    ("gox{BS}", "g"),
    ("gos{BS}x", "gx"),
    ("tieengs vieetj", "tiếng việt"),
    ("nguoiwf dduwowngf", "người đường"),
    ("tieengs{BS}{BS}", "tiế"),
    ("gox.", "gõ."),
    ("gox;", "gõ;"),
    # VS Code closes the bracket itself, as it does without a keyboard tool
    ("gox(", "gõ()"),
]
# word-based suggestions come from these words: "go" and "ti" offer them in the popup
SEED = "gotest_value = 1\ngoogle_thing = 2\ntiengviet_name = 3\nnguoidung_id = 4\n"

tmp = pathlib.Path(tempfile.mkdtemp(prefix="govn-vsc-"))
profile = tmp / "profile"
(profile / "User").mkdir(parents=True)
(profile / "User" / "settings.json").write_text(json.dumps({
    "files.autoSave": "afterDelay",
    "files.autoSaveDelay": 100,
    "workbench.startupEditor": "none",
    "update.mode": "none",
    "telemetry.telemetryLevel": "off",
    "security.workspace.trust.enabled": False,
    "editor.quickSuggestionsDelay": 0,
    "files.eol": "\n",
}), encoding="utf-8")


def find_pid():
    ps = ("Get-CimInstance Win32_Process -Filter \"Name='Code.exe'\" | "
          f"Where-Object {{ $_.CommandLine -like '*{tmp.name}*' -and $_.CommandLine -notlike '*--type=*' }} | "
          "Select-Object -First 1 -ExpandProperty ProcessId")
    out = subprocess.run(["pwsh", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout.strip()
    return int(out) if out else None


def send(pid, keys):
    r = subprocess.run([sendkeys, str(pid), keys], capture_output=True)
    if r.returncode:
        raise SystemExit(f"sendkeys failed: {r.stderr.decode(errors='replace')}")


failed = total = 0
proc = kb = None
try:
    for name in ("test.js", "test.py", "notes.txt"):
        f = tmp / name
        f.write_text(SEED, encoding="utf-8", newline="\n")
    proc = subprocess.Popen([code, f"--user-data-dir={profile}", f"--extensions-dir={tmp / 'ext'}", "--disable-extensions",
                             "--new-window"])
    time.sleep(18)  # a cold start can drop keys typed too early
    pid = find_pid()
    if not pid:
        raise SystemExit("VS Code process not found")

    for name in ("test.js", "test.py", "notes.txt"):
        f = tmp / name
        if kb:
            kb.kill()
            kb.wait()
        # Opening the file through the CLI is what moves focus into its editor.
        subprocess.run([code, f"--user-data-dir={profile}", "--reuse-window", str(f)], capture_output=True)
        time.sleep(2.5)
        send(pid, "{CEND}")

        # Control, keyboard not running yet: Tab accepts a suggestion only if the popup really is showing.
        send(pid, "got")
        time.sleep(0.6)
        send(pid, "{TAB}")
        time.sleep(0.5)
        last = f.read_text(encoding="utf-8").split("\n")[-1]
        print(f"[{name}] đối chứng: 'got'+Tab ra {last!r} =>", "CÓ popup gợi ý" if last == "gotest_value" else "KHÔNG có popup")
        send(pid, "{ESC}{ENTER}")

        kb = subprocess.Popen([govn, "--only-pid", str(pid), "--accept-injected", "--duration-secs", "150",
                               "--suggestion-fix", fix], stdout=subprocess.DEVNULL)
        time.sleep(0.5)
        for keys, want in CASES:
            send(pid, keys)
            time.sleep(0.7)
            got = f.read_text(encoding="utf-8").split("\n")[-1]
            total += 1
            if got == want:
                print(f"ok    [{name}] {keys!r}: {got!r}")
            else:
                failed += 1
                print(f"FAIL  [{name}] {keys!r}: muốn {want!r}, ra {got!r}")
            send(pid, "{ESC}{ENTER}")
            time.sleep(0.15)
finally:
    if kb:
        kb.kill()
    if proc:
        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True)
    pid2 = find_pid()
    if pid2:
        subprocess.run(["taskkill", "/PID", str(pid2), "/T", "/F"], capture_output=True)
    time.sleep(1.5)
    shutil.rmtree(tmp, ignore_errors=True)
print(f"đạt {total - failed}/{total}")
sys.exit(1 if failed else 0)
