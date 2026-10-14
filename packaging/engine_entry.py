"""PyInstaller entry point for the packaged OpenDemand sidecar.

The frozen binary accepts exactly the same CLI as ``python -m opendemand_engine``:
``opendemand-engine --job-dir TEMP``.

``multiprocessing.freeze_support()`` must run first: the engine's worker pool
starts processes with the spawn method, and PyInstaller re-launches this frozen
binary for each worker.
"""

from __future__ import annotations

import multiprocessing

from opendemand_engine.cli import main

if __name__ == "__main__":
    multiprocessing.freeze_support()
    raise SystemExit(main())
