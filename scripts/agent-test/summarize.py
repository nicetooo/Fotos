#!/usr/bin/env python3
"""Print a readable verdict for an agent self-test report. Exit 0 if it passed."""

import json
import sys


def main() -> int:
    with open(sys.argv[1]) as f:
        report = json.load(f)

    print(f"\nscenario: {report['scenario']}   elapsed: {report['elapsedMs']} ms   "
          f"viewport: {report['viewport']['w']}x{report['viewport']['h']} @{report['viewport']['dpr']}x")
    if report.get("boot"):
        print("boot marks (ms): " + ", ".join(f"{k}={v}" for k, v in report["boot"].items()))

    print(f"\n{'step':<22} {'ok':<4} {'fps':>6} {'p95':>7} {'max':>7} {'long':>5}  detail")
    for step in report["steps"]:
        frames = step.get("frames") or {}
        detail = step.get("detail")
        if isinstance(detail, dict):
            detail = ", ".join(f"{k}={v}" for k, v in detail.items() if not isinstance(v, (dict, list)))
        print(f"{step['name']:<22} {'yes' if step['ok'] else 'NO':<4} "
              f"{frames.get('fps', ''):>6} {frames.get('p95', ''):>7} {frames.get('maxMs', ''):>7} "
              f"{frames.get('long', ''):>5}  {detail or ''}")

    if report["errors"]:
        print("\nerrors:")
        for err in report["errors"]:
            print("  - " + err)

    print("\nRESULT: " + ("PASSED" if report["passed"] else "FAILED"))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
