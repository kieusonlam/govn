"""Find and shrink a key sequence that crashes the C++ oracle, then replay it on the Rust engine.

usage: find_crash.py <oracle.exe> <trace.exe> <difftest.py> <seed>
Regenerates the difftest script for the seed, finds the line that crashes, and minimises it.
"""
import re
import runpy
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
oracle, trace, difftest, seed = sys.argv[1:5]

# Reuse difftest's generator without running its comparison: stop it at the first subprocess call.
class Stop(Exception):
    pass

captured = {}
real_run = subprocess.run


def fake_run(cmd, input=None, capture_output=None):
    captured["payload"] = input
    raise Stop


subprocess.run = fake_run
sys.argv = [difftest, oracle, trace, "60000", seed]
try:
    runpy.run_path(difftest, run_name="__main__")
except Stop:
    pass
subprocess.run = real_run

lines = captured["payload"].decode().split("\n")


def crashes(script_lines):
    r = subprocess.run([oracle], input=("\n".join(script_lines) + "\n").encode(), capture_output=True)
    return r.returncode != 0


# bisect on prefix length: smallest prefix that crashes
lo, hi = 0, len(lines)
assert crashes(lines), "no crash for this seed"
while hi - lo > 1:
    mid = (lo + hi) // 2
    if crashes(lines[:mid]):
        hi = mid
    else:
        lo = mid
bad = lines[hi - 1]
cfg = next(l for l in reversed(lines[:hi]) if l.startswith("CFG"))
print("config:", cfg)

# does the line alone crash from a fresh engine?
alone = crashes([cfg, bad])
print("crashes alone from fresh engine:", alone, "| tokens:", len(bad.split()))
if not alone:
    # state leaks across lines: keep a window of previous lines
    for w in (2, 5, 20, 100, 1000):
        ctx = [l for l in lines[max(0, hi - w):hi] if not l.startswith("CFG")]
        if crashes([cfg] + ctx):
            print("needs previous lines, window", w)
            bad = " ".join(ctx)
            break

# delta-debug tokens
toks = bad.split()
n = 2
while len(toks) >= 2:
    size = max(1, len(toks) // n)
    reduced = False
    for i in range(0, len(toks), size):
        cand = toks[:i] + toks[i + size:]
        if cand and crashes([cfg, " ".join(cand)]):
            toks = cand
            n = max(n - 1, 2)
            reduced = True
            break
    if not reduced:
        if size == 1:
            break
        n = min(n * 2, len(toks))

print("minimal tokens:", len(toks))
print("script:", " ".join(toks))
printable = "".join(chr(int(t, 16)) if re.fullmatch(r"[0-9a-f]+", t) and 32 <= int(t, 16) < 127 else f"<{t}>" for t in toks)
print("as text:", printable)
r = subprocess.run([oracle], input=f"{cfg}\n{' '.join(toks)}\n".encode(), capture_output=True)
print("C++  rc:", r.returncode, hex(r.returncode & 0xFFFFFFFF), "| output lines:", len(r.stdout.splitlines()))
r = subprocess.run([trace], input=f"{cfg}\n{' '.join(toks)}\n".encode(), capture_output=True)
print("Rust rc:", r.returncode, "| output lines:", len(r.stdout.splitlines()))
