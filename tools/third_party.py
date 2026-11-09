"""Writes the licence texts of every crate linked into govn.exe, for shipping next to it.

Usage: python tools/third_party.py <output file>

Takes the normal (not build or dev) dependencies of the `govn` package for the Windows target and
copies each crate's LICENSE / COPYING / NOTICE files. Crates that ship no such file are listed
with their licence expression.
"""
import json
import subprocess
import sys
from pathlib import Path

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

NAMES = ("license", "licence", "copying", "notice", "unlicense", "copyright")


def main(out: str) -> None:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"],
        check=True, capture_output=True, text=True, encoding="utf-8").stdout)
    packages = {(p["name"], p["version"]): p for p in meta["packages"]}
    workspace = {(p["name"], p["version"]) for p in meta["packages"] if p["id"] in meta["workspace_members"]}
    # cargo tree reports what is really built with the selected features; metadata over-reports.
    tree = subprocess.run(
        ["cargo", "tree", "-p", "govn", "-e", "normal", "--target", "x86_64-pc-windows-msvc",
         "--prefix", "none", "--format", "{p}"],
        check=True, capture_output=True, text=True, encoding="utf-8").stdout
    seen = set()
    for line in tree.splitlines():
        bits = line.split()
        if len(bits) >= 2 and bits[1].startswith("v"):
            seen.add((bits[0], bits[1][1:]))

    parts, missing = [], []
    for p in sorted((packages[i] for i in seen - workspace), key=lambda p: (p["name"], p["version"])):
        folder = Path(p["manifest_path"]).parent
        files = sorted(f for f in folder.iterdir() if f.is_file() and f.name.lower().startswith(NAMES))
        if p.get("license_file"):
            extra = folder / p["license_file"]
            if extra.is_file() and extra not in files:
                files.append(extra)
        head = f"{p['name']} {p['version']} ({p.get('license') or 'see below'})"
        if not files:
            missing.append(head)
            continue
        for f in files:
            text = f.read_text(encoding="utf-8", errors="replace").strip()
            parts.append(f"{'=' * 78}\n{head}\n{f.name}\n{'=' * 78}\n\n{text}\n")

    intro = (
        "govn.exe includes the following third-party crates. Their licence files follow.\n"
        "Slint is used under the GNU General Public License version 3 (see LICENSE).\n\n"
    )
    tail = ""
    if missing:
        tail = (f"{'=' * 78}\nCrates without a licence file in their package. Licence texts by identifier:\n"
                "https://spdx.org/licenses/ (Slint: the GPL-3.0 text in LICENSE)\n\n" + "\n".join(missing) + "\n")
    Path(out).write_text(intro + "\n".join(parts) + tail, encoding="utf-8", newline="\n")
    print(f"{len(seen - workspace)} crates, {len(parts)} files, {len(missing)} without a file -> {out}")


if __name__ == "__main__":
    main(sys.argv[1])
