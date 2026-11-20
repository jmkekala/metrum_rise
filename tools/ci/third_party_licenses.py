#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Writes the licence texts of every Rust crate linked into the game library (docs/release.md).

The crates' MIT, Apache-2.0, BSD, Zlib and MPL-2.0 licences require their texts and copyright
notices to travel with a binary. This collects them from the crate sources Cargo downloaded:
every crate reachable from the game crate through normal dependencies, on every platform, so one
file serves the Linux, Windows and macOS packages. Development and build-only dependencies are
not linked into the library and are left out.

Identical texts are printed once with the crates that share them. A crate that ships no licence
file gets the standard text of its declared licence from `tools/ci/license_texts/`, with its
authors and repository. The run fails when a crate has neither, so a new dependency cannot ship
without a notice.

    tools/ci/third_party_licenses.py <rust-dir> <output-file>
"""

import json
from pathlib import Path
import re
import subprocess
import sys

LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|UNLICENSE)", re.IGNORECASE)
TEXTS = Path(__file__).resolve().parent / "license_texts"
RULE = "=" * 78

MIT = """MIT License

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""


def linked_packages(metadata):
    """The packages reachable from the root crate through normal dependency edges."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    seen = {root}
    stack = [root]
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            normal = any(kind["kind"] is None for kind in dep["dep_kinds"])
            if normal and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    seen.discard(root)
    return sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"]))


def shipped_texts(package):
    """The licence and notice files in the crate's source directory, in name order."""
    directory = Path(package["manifest_path"]).parent
    files = sorted(
        f for f in directory.iterdir() if f.is_file() and LICENSE_FILE.match(f.name)
    )
    return [f.read_text(encoding="utf-8", errors="replace").strip() + "\n" for f in files]


def standard_text(package):
    """The standard text of the first declared alternative this script has, or None."""
    declared = package.get("license") or ""
    if " AND " in declared or " WITH " in declared:
        return None
    for option in re.split(r"\s+OR\s+|/", declared.strip("() ")):
        option = option.strip()
        if option == "MIT":
            holders = ", ".join(
                re.sub(r"\s*<[^>]*>", "", author) for author in package["authors"]
            ) or f"the {package['name']} authors"
            return MIT.format(holders=holders)
        path = TEXTS / f"{option}.txt"
        if path.is_file():
            return path.read_text(encoding="utf-8").strip() + "\n"
    return None


def label(package):
    """One line naming a crate: name, version, declared licence and where its source is."""
    source = package.get("repository") or f"https://crates.io/crates/{package['name']}"
    return f"{package['name']} {package['version']} ({package.get('license')}) {source}"


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__.strip().splitlines()[-1].strip())
    rust_dir, output = sys.argv[1], Path(sys.argv[2])
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=rust_dir,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
    )
    packages = linked_packages(metadata)

    # Text -> the crates that carry it, in first-use order (packages are sorted by name).
    users = {}
    missing = []
    for package in packages:
        texts = shipped_texts(package)
        if not texts:
            fallback = standard_text(package)
            if fallback is None:
                missing.append(label(package))
                continue
            texts = [fallback]
        for text in texts:
            users.setdefault(text, []).append(label(package))
    if missing:
        sys.exit(
            "No licence file and no standard text in tools/ci/license_texts/ for:\n  "
            + "\n  ".join(missing)
        )

    parts = [
        "Third-party Rust crates in Metrum Rise\n\n"
        "The game library is built from the crates below. Their licence texts follow,\n"
        "each under the list of crates it applies to.\n\n"
        f"Crates ({len(packages)}):\n" + "".join(f"  {label(p)}\n" for p in packages)
    ]
    for text, crates in users.items():
        parts.append("Applies to:\n" + "".join(f"  {crate}\n" for crate in crates) + "\n" + text)
    output.write_text(f"\n{RULE}\n\n".join(parts), encoding="utf-8")
    print(f"{output}: {len(packages)} crates, {len(users)} licence texts")


if __name__ == "__main__":
    main()
