"""Command line entry point: ``python -m opendemand_engine --job-dir TEMP``.

The job directory is validated before ``hello``; heavy imports and the whole
engine module load only happen afterwards.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        prog="opendemand_engine",
        description="OpenDemand forecasting sidecar (opendemand.sidecar/1).",
    )
    parser.add_argument(
        "--job-dir",
        required=True,
        help="job directory holding the input Arrow files and receiving the outputs",
    )
    args = parser.parse_args(argv)

    job_dir = Path(args.job_dir)
    if not job_dir.is_dir():
        print(
            f"opendemand-engine: --job-dir does not exist or is not a directory: {args.job_dir}",
            file=sys.stderr,
        )
        return 2

    stdout, stdin = sys.stdout, sys.stdin
    for stream, kwargs in (
        (stdout, {"encoding": "utf-8", "newline": "\n"}),
        (stdin, {"encoding": "utf-8"}),
    ):
        try:
            stream.reconfigure(**kwargs)
        except (AttributeError, ValueError, OSError):
            pass

    from .engine import run_job

    return run_job(job_dir, stdin, stdout, sys.stderr)


if __name__ == "__main__":
    sys.exit(main())
