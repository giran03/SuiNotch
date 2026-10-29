"""Packs brand/icons/app-<size>.png into brand/icons/suinotch.ico (PNG-compressed entries).

Sizes up to 32 px come from the small-size cut, larger ones from the full mark; see build_brand.py.
Run: python -I brand/pack_ico.py
"""

import struct
from pathlib import Path

ICONS = Path(__file__).resolve().parent / "icons"
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def main():
    images = [(s, (ICONS / f"app-{s}.png").read_bytes()) for s in SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    out = ICONS / "suinotch.ico"
    out.write_bytes(header + entries + blobs)
    print(f"wrote {out} ({len(images)} sizes, {out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
