"""Zip the files in ./pack. Used by GitHub Actions, including Windows, which has no zip."""

import sys
import zipfile
from pathlib import Path

out = Path(sys.argv[1])
root = Path("pack")
with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(root.iterdir()):
        info = zipfile.ZipInfo(path.name)
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = (path.stat().st_mode & 0xFFFF) << 16
        archive.writestr(info, path.read_bytes())
