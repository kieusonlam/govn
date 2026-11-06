"""Real-Chrome test of typing with inline autocomplete. Uses a throwaway profile and local pages only.

usage: test_chrome.py <chrome.exe> <govn.exe> <sendkeys.exe> [--no-fix-flag ...extra govn args]
The omnibox is seeded with a local URL starting with "go", so typing "go" gets an inline suggestion
exactly like the reported bug. The omnibox is read through UI Automation.
"""
import http.server
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
chrome, govn, sendkeys = sys.argv[1:4]
extra = sys.argv[4:]


PAGES = {
    "/textarea": "<textarea id=f autofocus></textarea>",
    "/input": "<input id=f autofocus>",
    "/editable": "<div id=f contenteditable style='min-height:3em'></div>",
}
SCRIPT = """<script>
const f = document.getElementById('f'); f.focus();
let n = 0;
f.addEventListener('input', () => fetch('/v?n=' + (++n), {method: 'POST', body: f.value !== undefined ? f.value : f.innerText}));
</script>"""
latest = {"n": -1, "v": ""}


class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.path.split("=")[1])
        v = self.rfile.read(int(self.headers["Content-Length"])).decode("utf-8")
        if n > latest["n"]:
            latest.update(n=n, v=v)
        self.send_response(204)
        self.end_headers()

    def do_GET(self):
        page = PAGES.get(self.path)
        body = ("<!doctype html><meta charset=utf-8><title>test</title>" + (page + SCRIPT if page else "ok")).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


srv = socketserver.TCPServer(("127.0.0.1", 0), H)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

READ_OMNIBOX = r"""
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$pid_ = %d
$root = [Windows.Automation.AutomationElement]::RootElement
$c = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty, $pid_)
$w = $root.FindFirst([Windows.Automation.TreeScope]::Children, $c)
$e = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
$b = $w.FindFirst([Windows.Automation.TreeScope]::Descendants, $e)
$v = $b.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
[Console]::OutputEncoding = [Text.Encoding]::UTF8
[Console]::Out.Write($v)
"""


def read_omnibox(pid):
    r = subprocess.run(["pwsh", "-NoProfile", "-Command", READ_OMNIBOX % pid], capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def send(pid, keys):
    r = subprocess.run([sendkeys, str(pid), keys], capture_output=True)
    if r.returncode:
        raise SystemExit(f"sendkeys failed: {r.stderr.decode(errors='replace')}")


def find_pid(profile):
    ps = ("Get-CimInstance Win32_Process -Filter \"Name='chrome.exe'\" | "
          f"Where-Object {{ $_.CommandLine -like '*{profile}*' -and $_.CommandLine -notlike '*--type=*' }} | "
          "Select-Object -First 1 -ExpandProperty ProcessId")
    return int(subprocess.run(["pwsh", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout.strip())


CASES = [
    # keys, expected omnibox text. Autocomplete suffix is cut off by reading after Delete.
    ("gox", "gõ"),
    ("gox tieengs vieetj thuwr", "gõ tiếng việt thử"),
    ("gos", "gó"),
    ("gofo", None),  # word continues after a tone; only checks nothing crashes
    ("gooj", "gộ"),
    ("goxx", "gox"),
    ("gox{BS}", "g"),
    ("tieengs{BS}{BS}", "tiế"),
    ("gos{BS}x", "gx"),
    ("xin chaof cacs banj", "xin chào các bạn"),
]

profile = tempfile.mkdtemp(prefix="govn-chrome-")
url = f"http://gotest.localhost:{port}/"
proc = subprocess.Popen([chrome, f"--user-data-dir={profile}", "--no-first-run", "--no-default-browser-check",
                         "--disable-sync", "--disable-extensions", "--new-window", "about:blank"])
kb = None
failed = 0
try:
    time.sleep(4)
    pid = find_pid(profile)
    # Seed history by typing the URL, twice, so it becomes an inline suggestion. Keyboard is not running yet.
    for _ in range(2):
        send(pid, "{L}" + url + "{ENTER}")
        time.sleep(1.5)
    send(pid, "{L}go")
    time.sleep(0.6)
    seen = read_omnibox(pid)
    print("kiểm tra gợi ý: gõ 'go' thấy", repr(seen), "=>", "CÓ gợi ý inline" if seen.startswith("gotest") else "KHÔNG có gợi ý")
    send(pid, "{ESC}{ESC}")

    kb = subprocess.Popen([govn, "--only-pid", str(pid), "--accept-injected", "--duration-secs", "120"] + extra,
                          stdout=subprocess.DEVNULL)
    time.sleep(0.5)
    for keys, want in CASES:
        send(pid, "{L}" + keys)
        time.sleep(0.5)
        # Delete removes whatever suggestion is still selected, leaving only what was typed.
        send(pid, "{DEL}")
        time.sleep(0.25)
        got = read_omnibox(pid)
        if want is None:
            print(f"info  {keys!r}: {got!r}")
        elif got == want:
            print(f"ok    {keys!r}: {got!r}")
        else:
            failed += 1
            print(f"FAIL  {keys!r}: muốn {want!r}, ra {got!r}")
        send(pid, "{ESC}{ESC}")

    # Fields inside web pages. The keyboard is stopped while navigating, so the URL is typed as is.
    PAGE_CASES = [
        ("tieengs vieetj", "tiếng việt"),
        ("nguoiwf dduwowngf", "người đường"),
        ("gox", "gõ"),
        ("toaf{BS}", "tò"),
        ("tieengs{BS}{BS}", "tiế"),
        ("xin chaof cacs banj.", "xin chào các bạn."),
    ]
    for path in PAGES:
        kb.kill()
        kb.wait()
        send(pid, "{L}" + f"http://gotest.localhost:{port}{path}" + "{ENTER}")
        time.sleep(1.5)
        kb = subprocess.Popen([govn, "--only-pid", str(pid), "--accept-injected", "--duration-secs", "120"] + extra,
                              stdout=subprocess.DEVNULL)
        time.sleep(0.5)
        for keys, want in PAGE_CASES:
            latest.update(n=-1, v="")
            send(pid, keys)
            time.sleep(0.4)
            got = latest["v"].replace(chr(0xA0), " ").rstrip(chr(10))
            label = f"{path} {keys!r}"
            if got == want:
                print(f"ok    {label}: {got!r}")
            else:
                failed += 1
                print(f"FAIL  {label}: muốn {want!r}, ra {got!r}")
            send(pid, "{A}{DEL}")
            time.sleep(0.25)
finally:
    if kb:
        kb.kill()
    subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True)
    time.sleep(1)
    shutil.rmtree(profile, ignore_errors=True)
print("thất bại:", failed)
sys.exit(1 if failed else 0)
