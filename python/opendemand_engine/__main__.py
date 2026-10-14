"""``python -m opendemand_engine`` entry point."""

from __future__ import annotations

import multiprocessing
import sys

from .cli import main

if __name__ == "__main__":
    # Frozen-spawn safety: the engine runs a bounded worker pool, so the
    # multiprocessing bootstrap must run before any engine work on Windows and
    # inside the PyInstaller bundle.
    multiprocessing.freeze_support()
    sys.exit(main())
