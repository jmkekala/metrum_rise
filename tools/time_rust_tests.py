#!/usr/bin/env python3
"""Time individual Rust lib tests and print the slowest exact test names."""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = REPO_ROOT / "rust" / "Cargo.toml"


def run_command(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def list_lib_tests(manifest_path: Path) -> list[str]:
    result = run_command(
        [
            "cargo",
            "test",
            "--manifest-path",
            str(manifest_path),
            "--lib",
            "--",
            "--list",
        ]
    )
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)

    tests = []
    for line in result.stdout.splitlines():
        if line.endswith(": test"):
            tests.append(line[: -len(": test")])
    return tests


def matches_filters(test_name: str, filters: list[str]) -> bool:
    return all(pattern in test_name for pattern in filters)


def time_one_test(manifest_path: Path, test_name: str) -> tuple[float, int, str]:
    command = [
        "cargo",
        "test",
        "--manifest-path",
        str(manifest_path),
        "--lib",
        test_name,
        "--",
        "--exact",
        "--quiet",
    ]
    started = time.perf_counter()
    result = run_command(command)
    elapsed_s = time.perf_counter() - started
    output = result.stdout + result.stderr
    return elapsed_s, result.returncode, output


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Rank Rust lib tests by exact per-test wall time."
    )
    parser.add_argument(
        "filters",
        nargs="*",
        help="Only time tests whose full names contain every filter string.",
    )
    parser.add_argument(
        "--manifest-path",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="Cargo manifest path. Defaults to rust/Cargo.toml.",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=20,
        help="Number of slow tests to print.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest_path = args.manifest_path
    if not manifest_path.is_absolute():
        manifest_path = REPO_ROOT / manifest_path

    tests = [
        test_name
        for test_name in list_lib_tests(manifest_path)
        if matches_filters(test_name, args.filters)
    ]
    if not tests:
        sys.stderr.write("No tests matched the requested filters.\n")
        return 1

    sys.stderr.write(f"Timing {len(tests)} Rust lib tests")
    if args.filters:
        sys.stderr.write(f" matching {' '.join(args.filters)!r}")
    sys.stderr.write("...\n")

    timings: list[tuple[float, str]] = []
    total_started = time.perf_counter()
    for index, test_name in enumerate(tests, start=1):
        elapsed_s, returncode, output = time_one_test(manifest_path, test_name)
        if returncode != 0:
            sys.stderr.write(output)
            sys.stderr.write(f"FAILED after {elapsed_s:.3f}s: {test_name}\n")
            return returncode
        timings.append((elapsed_s, test_name))
        sys.stderr.write(f"[{index}/{len(tests)}] {elapsed_s:8.3f}s  {test_name}\n")

    total_elapsed_s = time.perf_counter() - total_started
    timings.sort(reverse=True)
    print(f"Timed {len(timings)} tests in {total_elapsed_s:.3f}s")
    print(f"Slowest {min(args.top, len(timings))} tests:")
    for elapsed_s, test_name in timings[: args.top]:
        print(f"{elapsed_s:8.3f}s  {test_name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
