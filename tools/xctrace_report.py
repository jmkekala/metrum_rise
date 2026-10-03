#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Text report for an Instruments Time Profiler trace (macOS `xctrace`).

Exports the trace's `time-profile` table (or reads an existing XML export), resolves the
id/ref de-duplication `xctrace export` uses, demangles Rust v0 symbols with `rustfilt` when it is
installed, and prints per-thread sample totals plus top self/inclusive functions and binaries.

Example:
    xcrun xctrace record --template 'Time Profiler' --output run.trace --launch -- <cmd>
    tools/xctrace_report.py run.trace --window 37.6:38.6 --thread-filter Main
"""

import argparse
import collections
import os
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET


def export_xml(trace_path):
    xpath = '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]'
    result = subprocess.run(
        ["xcrun", "xctrace", "export", "--input", trace_path, "--xpath", xpath],
        check=True, capture_output=True,
    )
    return result.stdout


def parse_samples(xml_bytes):
    """Yields (time_s, thread_name, weight_ns, frames) with frames leaf-first as (name, binary)."""
    by_id = {}

    def resolve(element):
        ref = element.get("ref")
        if ref is not None:
            return by_id[ref]
        if element.get("id") is not None:
            by_id[element.get("id")] = element
        return element

    def register_tree(element):
        # Children of a fresh element can define ids that later rows reference.
        for child in element.iter():
            if child.get("id") is not None and child.get("ref") is None:
                by_id[child.get("id")] = child

    root = ET.fromstring(xml_bytes)
    for row in root.iter("row"):
        register_tree(row)
        time_s = thread = weight = None
        frames = []
        for field in row:
            node = resolve(field)
            if field.tag == "sample-time":
                time_s = int(node.text) / 1e9
            elif field.tag == "thread":
                thread = node.get("fmt", "?")
            elif field.tag == "weight":
                weight = int(node.text)
            elif field.tag in ("backtrace", "tagged-backtrace"):
                # Time Profiler wraps the stack in a tagged-backtrace; either may be a ref.
                if node.tag == "tagged-backtrace":
                    node = resolve(node.find("backtrace"))
                for frame in node.findall("frame"):
                    frame = resolve(frame)
                    binary = frame.find("binary")
                    binary_name = resolve(binary).get("name", "?") if binary is not None else "?"
                    frames.append((frame.get("name", "?"), binary_name))
        if time_s is not None and frames:
            yield time_s, thread or "?", weight or 1_000_000, frames


def demangler(names):
    rust = sorted({n for n in names if n.startswith("_R") or n.startswith("__R")})
    mapping = {}
    tool = shutil.which("rustfilt") or shutil.which("rustfilt", path=os.path.expanduser("~/.cargo/bin"))
    if rust and tool:
        out = subprocess.run([tool], input="\n".join(rust), capture_output=True, text=True).stdout
        mapping = dict(zip(rust, out.splitlines()))
    return lambda name: mapping.get(name, name)


def short(name, width):
    # Drop generic arguments so monomorphised copies of one function read as one line.
    depth, out = 0, []
    for ch in name:
        if ch == "<" and out and out[-1] not in " (<":
            depth += 1
            continue
        if ch == ">" and depth:
            depth -= 1
            continue
        if not depth:
            out.append(ch)
    text = re.sub(r"::\{closure#\d+\}", "::{closure}", "".join(out))
    return text if len(text) <= width else text[: width - 1] + "…"


def thread_label(fmt):
    # "Main Thread (0x1a2b) (Godot, pid: 123)" -> "Main Thread (0x1a2b)"; unnamed threads keep the tid.
    return re.sub(r"\s+", " ", re.sub(r"\s*\([^()]*pid: \d+\)\s*$", "", fmt)).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("trace", help=".trace bundle or an XML export of its time-profile table")
    parser.add_argument("--window", help="START:END seconds from trace start")
    parser.add_argument("--thread-filter", help="regex on thread label; keeps matching threads only")
    parser.add_argument("--exclude", help="regex on function names; drops samples whose stack contains one")
    parser.add_argument("--top", type=int, default=30)
    parser.add_argument("--width", type=int, default=110)
    parser.add_argument("--save-xml", help="also write the raw export here")
    args = parser.parse_args()

    if args.trace.endswith(".xml"):
        xml_bytes = open(args.trace, "rb").read()
    else:
        xml_bytes = export_xml(args.trace)
        if args.save_xml:
            open(args.save_xml, "wb").write(xml_bytes)

    window = tuple(float(v) for v in args.window.split(":")) if args.window else None
    thread_re = re.compile(args.thread_filter) if args.thread_filter else None
    exclude_re = re.compile(args.exclude) if args.exclude else None

    threads = collections.Counter()
    self_ns = collections.Counter()
    incl_ns = collections.Counter()
    binary_ns = collections.Counter()
    total = 0
    first = last = None
    for time_s, thread, weight, frames in parse_samples(xml_bytes):
        if window and not (window[0] <= time_s < window[1]):
            continue
        label = thread_label(thread)
        if thread_re and not thread_re.search(label):
            continue
        if exclude_re and any(exclude_re.search(name) for name, _ in frames):
            continue
        first = time_s if first is None else min(first, time_s)
        last = time_s if last is None else max(last, time_s)
        total += weight
        threads[label] += weight
        self_ns[frames[0]] += weight
        binary_ns[frames[0][1]] += weight
        for frame in set(frames):
            incl_ns[frame] += weight

    if not total:
        sys.exit("no samples matched")
    demangle = demangler({name for name, _ in incl_ns})
    span = (last - first) if last and first is not None and last > first else 0.0

    def pct(ns):
        return 100.0 * ns / total

    print(f"samples: {total / 1e6:.0f} ms of CPU over {span:.2f} s wall"
          f"{' window ' + args.window if args.window else ''}")
    print("\n== CPU by thread")
    for label, ns in threads.most_common(args.top):
        print(f"{pct(ns):6.1f}%  {ns / 1e6:9.0f} ms  {label}")
    print("\n== Self time by binary")
    for binary, ns in binary_ns.most_common(15):
        print(f"{pct(ns):6.1f}%  {binary}")
    for title, counter in (("Self time", self_ns), ("Inclusive time", incl_ns)):
        print(f"\n== {title} by function")
        for (name, binary), ns in counter.most_common(args.top):
            print(f"{pct(ns):6.1f}%  {short(demangle(name), args.width)}  [{binary}]")


if __name__ == "__main__":
    main()
