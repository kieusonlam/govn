"""Differential test: Rust engine vs the original C++ ukengine, on the same key scripts.

usage: difftest.py <oracle.exe> <trace.exe> [lines] [seed]
"""
import itertools
import os
import random
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
oracle, trace = sys.argv[1], sys.argv[2]
LINES = int(sys.argv[3]) if len(sys.argv) > 3 else 4000
SEED = int(sys.argv[4]) if len(sys.argv) > 4 else 20261106
rnd = random.Random(SEED)

ONSETS = "b c ch d dd g gh gi h k kh l m n ng ngh nh p ph qu r s t th tr v x".split() + [""]
VOWELS = ("a aa aw e ee i o oo ow u uw y ai ao au ay aau aay eo eu eeu ia ie iee ieu ieeu iu oa oaw oe oi ooi owi "
          "ua uaa ue uee ui uo uoo uow uwo uwow uu uy uwa uwi uwu ye yee yeu yeeu oai oay oeo uay uaay uoi uooi uowi "
          "uwoi uwowi uwou uwowu uya uye uyee uyu").split()
CODAS = "c ch m n ng nh p t".split() + ["", "", ""]
TELEX_TONES = list("sfrxjz")
VNI_TONES = list("123450")
VNI_MARKS = list("6789")


def telex_word():
    w = rnd.choice(ONSETS) + rnd.choice(VOWELS)
    coda = rnd.choice(CODAS)
    tone = rnd.choice(TELEX_TONES + [""])
    mode = rnd.random()
    if mode < 0.5:
        w += coda + tone
    elif mode < 0.8:
        w += tone + coda
    else:  # stray marks and repeats: exercises undo paths
        w += coda + rnd.choice(["w", "aa", "oo", "ee", "dd", "ww", "ss", "ff", "uw", "ow", "w" + tone])
    return w


def vni_word():
    plain = rnd.choice(ONSETS).replace("dd", "d9") + rnd.choice(VOWELS)
    for a, b in (("aa", "a6"), ("aw", "a8"), ("ee", "e6"), ("oo", "o6"), ("ow", "o7"), ("uw", "u7")):
        plain = plain.replace(a, b)
    coda = rnd.choice(CODAS)
    tone = rnd.choice(VNI_TONES + [""])
    mode = rnd.random()
    if mode < 0.6:
        return plain + coda + tone
    if mode < 0.8:
        return plain + tone + coda
    return plain + coda + rnd.choice(VNI_MARKS) + tone


def case(w):
    r = rnd.random()
    if r < 0.8:
        return w
    if r < 0.9:
        return w.capitalize()
    if r < 0.97:
        return w.upper()
    return "".join(c.upper() if rnd.random() < 0.5 else c for c in w)


def hexes(s):
    return [f"{ord(c):x}" for c in s]


# ASCII_ONLY avoids Latin-1 keys, which can crash the C++ original (e.g. g, 0xCD, d) and cut the comparison short
ASCII_ONLY = os.environ.get("ASCII_ONLY") == "1"
ALL_BYTES = [f"{i:x}" for i in range(1, 128 if ASCII_ONLY else 256)]
ASCII_PRINT = [f"{i:x}" for i in range(33, 127)]
LETTERS = [f"{ord(c):x}" for c in "aeiouyAEIOUYdDwWsfrxjzbcghklmnpqtv[]{}"]


def script_line(method):
    word = vni_word if method == 1 else telex_word
    toks = []
    kind = rnd.random()
    if kind < 0.55:  # text made of plausible words
        for _ in range(rnd.randint(1, 14)):
            toks += hexes(case(word()))
            r = rnd.random()
            if r < 0.75:
                toks.append("20")
            elif r < 0.85:
                toks.append(rnd.choice(["2e", "2c", "21", "3f", "2d", "28", "29", "27"]))
            elif r < 0.93:
                toks += ["BS"] * rnd.randint(1, 5)
            elif r < 0.97:
                toks.append("RST")
            else:
                toks.append(rnd.choice(["CAPS0", "CAPS1"]))
    elif kind < 0.8:  # letter soup
        for _ in range(rnd.randint(2, 60)):
            r = rnd.random()
            toks.append("BS" if r < 0.08 else "20" if r < 0.15 else rnd.choice(LETTERS))
    elif kind < 0.93:  # any byte, and some Unicode key codes
        for _ in range(rnd.randint(2, 60)):
            r = rnd.random()
            if r < 0.05:
                toks.append("BS")
            elif r < 0.1 and not ASCII_ONLY:
                toks.append(f"{rnd.randint(256, 0xffff):x}")
            elif r < 0.5:
                toks.append(rnd.choice(ALL_BYTES))
            else:
                toks.append(rnd.choice(ASCII_PRINT))
    else:  # long unbroken runs: wrap the 128-entry buffers
        for _ in range(rnd.randint(130, 400)):
            toks.append(rnd.choice(LETTERS) if rnd.random() < 0.97 else "20")
        # then walk back through and past the point where the buffers were shifted
        for _ in range(rnd.randint(0, 150)):
            toks.append("BS" if rnd.random() < 0.8 else rnd.choice(LETTERS))
    return " ".join(toks)


CONFIGS = list(itertools.product([0, 1, 5, 6], [1, 0], [0, 1], [1, 0], [0, 1]))
script, labels = [], []
for i in range(LINES):
    if i % 25 == 0:
        cfg = rnd.choice(CONFIGS)
        script.append("CFG " + " ".join(map(str, cfg)))
    script.append(script_line(cfg[0]))
payload = ("\n".join(script) + "\n").encode()

a = subprocess.run([oracle], input=payload, capture_output=True)
b = subprocess.run([trace], input=payload, capture_output=True)
print("oracle rc", a.returncode, "| rust rc", b.returncode)
if b.returncode:
    print(b.stderr.decode(errors="replace")[-1500:])
A, B = a.stdout.decode().split("\n"), b.stdout.decode().split("\n")
print("output lines:", len(A), len(B), "| configs used:", len(set(s for s in script if s.startswith("CFG"))))

# walk script and outputs in step, to report the first differences with their context
pos = 0
diffs = 0
keys = 0
for lineno, line in enumerate(script):
    toks = [line] if line.startswith("CFG") else line.split()
    for ti, t in enumerate(toks):
        keys += 1
        if pos >= len(A) or pos >= len(B):
            break
        if A[pos].strip() != B[pos].strip():
            diffs += 1
            if diffs <= 12:
                last_cfg = next(s for s in reversed(script[:lineno + 1]) if s.startswith("CFG"))
                ctx = " ".join(toks[max(0, ti - 25):ti + 1])
                print(f"DIFF #{diffs} line {lineno} tok {ti} [{last_cfg}]\n  ctx: {ctx}\n  c++ : {A[pos].strip()!r}\n  rust: {B[pos].strip()!r}")
        pos += 1
print(f"tokens compared: {keys:,} | differences: {diffs:,}")
sys.exit(1 if diffs or a.returncode or b.returncode or len(A) != len(B) else 0)
