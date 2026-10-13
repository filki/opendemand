"""Build the standalone sidecar with PyInstaller (onedir).

Usage: ``uv run python packaging/build_sidecar.py``

Output: ``packaging/sidecar-dist/opendemand-engine/`` containing the
``opendemand-engine`` executable. The Tauri bundle ships that directory as
the ``engine`` resource, and the desktop app resolves it from there.

PyInstaller follows the engine's real imports, so only the modules the engine
actually loads are frozen: pyarrow, numpy, statsforecast (with coreforecast),
utilsforecast and pandas. Heavy optional packages that StatsForecast can pull
in (matplotlib, Jupyter, pytest) are excluded.
"""

from __future__ import annotations

import importlib.metadata
import shutil
from pathlib import Path

import PyInstaller.__main__

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "packaging" / "sidecar-dist"
WORK = ROOT / "packaging" / "sidecar-build"

COPY_METADATA = [
    "statsforecast",
    "utilsforecast",
    "coreforecast",
    "numba",
    "llvmlite",
    "numpy",
    "pandas",
    "pyarrow",
]

EXCLUDES = [
    "matplotlib",
    "tkinter",
    "IPython",
    "jupyter",
    "notebook",
    "pytest",
    "sphinx",
]


def main() -> int:
    for directory in (DIST, WORK):
        shutil.rmtree(directory, ignore_errors=True)

    arguments = [
        str(ROOT / "packaging" / "engine_entry.py"),
        "--name",
        "opendemand-engine",
        "--onedir",
        "--noconfirm",
        "--clean",
        "--paths",
        str(ROOT / "python"),
        "--distpath",
        str(DIST),
        "--workpath",
        str(WORK),
        "--specpath",
        str(WORK),
    ]
    for package in COPY_METADATA:
        try:
            importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            continue
        arguments += ["--copy-metadata", package]
    for module in EXCLUDES:
        arguments += ["--exclude-module", module]

    PyInstaller.__main__.run(arguments)
    binary = DIST / "opendemand-engine" / "opendemand-engine"
    print(f"sidecar built: {binary if binary.exists() else DIST}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
