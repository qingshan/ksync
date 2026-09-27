#!/usr/bin/env python3
"""Run host WAF tests; optionally record and encode the real-Kindle tour."""
import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run(*args):
    subprocess.run(args, cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true", help="capture real Kindle frames and encode GIF/MP4")
    args = parser.parse_args()
    run("node", "tests/e2e/waf_e2e.js")
    run("node", "tests/e2e/browser_e2e.js")
    if args.record:
        run(sys.executable, "tests/e2e/kindle_demo.py")
        run(sys.executable, "tools/demo_build.py", "--readme")
    print("ksync E2E: all selected scenarios passed")


if __name__ == "__main__":
    main()
