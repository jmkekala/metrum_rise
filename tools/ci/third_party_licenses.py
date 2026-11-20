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

Before anything is written, every linked crate's declared licence is checked against
`ACCEPTED`, the licences that may be combined with the game's `GPL-3.0-or-later`. A crate
passes when one alternative of its licence expression is accepted in full. `--check` runs only
this check, for the test gate (`./run.sh --test`).

    tools/ci/third_party_licenses.py <rust-dir> <output-file>
    tools/ci/third_party_licenses.py --check <rust-dir>
"""

import json
from pathlib import Path
import re
import subprocess
import sys

LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|UNLICENSE)", re.IGNORECASE)
TEXTS = Path(__file__).resolve().parent / "license_texts"
RULE = "=" * 78

# Licences a linked crate may carry: each is compatible with GPL version 3 (FSF licence list).
# Add one only after checking that; GPL-2.0-only, SSPL, BUSL and the like never belong here.
ACCEPTED = {
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
}

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


def licence_accepted(expression):
    """Whether an SPDX expression can be satisfied with accepted licences only.

    `OR` needs one side, `AND` both. `X WITH <exception>` counts as `X`: an exception only adds
    permissions. `X+` (this version or later) counts as `X`. Cargo's old `A/B` means `A OR B`.
    """
    tokens = re.findall(r"\(|\)|[^\s()]+", (expression or "").replace("/", " OR "))
    position = 0

    def peek():
        return tokens[position] if position < len(tokens) else None

    def take():
        nonlocal position
        position += 1
        return tokens[position - 1]

    def term():
        if peek() is None:
            raise ValueError("incomplete expression")
        token = take()
        if token == "(":
            value = alternatives()
            if peek() != ")":
                raise ValueError("unbalanced brackets")
            take()
            return value
        if token in (")", "OR", "AND", "WITH"):
            raise ValueError(f"unexpected {token}")
        if peek() == "WITH":
            take()
            take()
        return token.removesuffix("+") in ACCEPTED

    def conjunction():
        value = term()
        while peek() == "AND":
            take()
            value = term() and value
        return value

    def alternatives():
        value = conjunction()
        while peek() == "OR":
            take()
            value = conjunction() or value
        return value

    try:
        value = alternatives()
    except (ValueError, IndexError):
        return False
    return value and position == len(tokens)


def rejected_licences(packages):
    """A line for each crate whose declared licence is missing or not acceptable."""
    return [
        f"{package['name']} {package['version']}: licence \"{package.get('license')}\""
        for package in packages
        if not licence_accepted(package.get("license"))
    ]


def label(package):
    """One line naming a crate: name, version, declared licence and where its source is."""
    source = package.get("repository") or f"https://crates.io/crates/{package['name']}"
    return f"{package['name']} {package['version']} ({package.get('license')}) {source}"


def main():
    arguments = sys.argv[1:]
    check_only = arguments[:1] == ["--check"]
    if len(arguments) != 2:
        sys.exit("usage: third_party_licenses.py <rust-dir> <output-file> | --check <rust-dir>")
    rust_dir = arguments[1] if check_only else arguments[0]
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=rust_dir,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
    )
    packages = linked_packages(metadata)

    rejected = rejected_licences(packages)
    if rejected:
        sys.exit(
            "Crates linked into the game whose licence is not on the accepted list\n"
            "(ACCEPTED in tools/ci/third_party_licenses.py; docs/release.md, Licence files):\n  "
            + "\n  ".join(rejected)
        )
    if check_only:
        print(f"Crate licences: {len(packages)} linked crates, all accepted.")
        return
    output = Path(arguments[1])

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
