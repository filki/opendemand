"""PyInstaller entry point for the packaged OpenDemand sidecar.

The frozen binary accepts exactly the same CLI as ``python -m opendemand_engine``:
``opendemand-engine --job-dir TEMP``.
"""

from __future__ import annotations

from opendemand_engine.cli import main

if __name__ == "__main__":
    raise SystemExit(main())
