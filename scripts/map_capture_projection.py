"""Exact subregions of one validated map/1.5 payload, not additional native reads.

Preserve original tile record bytes (including hidden/missing presence). Only
region metadata is rewritten. Callers retain the parent bytes and these source
links; a projected payload alone cannot attest its native acquisition.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import struct
from typing import Callable

import excavation_observer as native

MAX_TILES = 4096
MAX_BYTES = 575 + 20 * MAX_TILES
Guard = Callable[[], None]


def idle() -> None:
    """Pure use retains fixed structural bounds."""


def decode(raw: bytes, manifest: native.Manifest, region: native.Region,
           guard: Guard = idle) -> native.Capture:
    guard()
    native.require(type(region) is native.Region and type(manifest) is native.Manifest,
                   'exact map region and manifest required')
    region.__post_init__()
    manifest.__post_init__()
    native.require(region.volume <= MAX_TILES and type(raw) is bytes
                   and 1 <= len(raw) <= 575 + 20 * region.volume,
                   'map projection source exceeds bound')
    capture = native.decode_capture(raw, manifest, region)
    guard()
    return capture


@dataclass(frozen=True)
class Projection:
    raw: bytes
    parent_witness: str
    parent_sha256: str
    region: native.Region
    witness: str

    def json(self) -> dict:
        return {'policy': 'dfmcp.exact-map-subregion/1', 'region': self.region.json(),
                'parent_map_witness': self.parent_witness,
                'parent_capture_sha256': self.parent_sha256,
                'capture_sha256': hashlib.sha256(self.raw).hexdigest(),
                'map_witness': self.witness, 'capture_bytes': len(self.raw),
                'additional_native_read': False, 'acquisition_attested': False}


def project(raw: bytes, manifest: native.Manifest, parent: native.Region,
            target: native.Region, guard: Guard = idle) -> Projection:
    """Validate the full source before slicing; never pad unknown source cells."""
    capture = decode(raw, manifest, parent, guard)
    native.require(type(target) is native.Region, 'exact projection region required')
    target.__post_init__()
    native.require(all(a <= b and b + m <= a + n for a, n, b, m in
                       zip(parent.origin, parent.size, target.origin, target.size)),
                   'projection extends beyond the original complete capture')
    # DFMM1500 + clock/pause/site + u16 folder length + UTF-8 folder + dimensions.
    # Copy this entire prefix verbatim instead of synthesizing a second source.
    prefix = 35 + len(capture.folder.encode('utf-8'))
    offset = prefix + 28  # origin, size and cell count (seven big-endian u32s).
    records = []
    for tile in capture.tiles:
        guard()
        end = offset + (20 if tile.presence == 2 else 1)
        records.append((offset, end))
        offset = end
    native.require(offset == len(raw), 'map projection record boundaries differ')
    out = bytearray(raw[:prefix])
    out.extend(struct.pack('>7I', *target.origin, *target.size, target.volume))
    ox, oy, oz = parent.origin
    width, height, _ = parent.size
    tx, ty, tz = target.origin
    tw, th, tl = target.size
    for z in range(tz, tz + tl):
        for y in range(ty, ty + th):
            for x in range(tx, tx + tw):
                guard()
                start, end = records[((z - oz) * height + y - oy) * width + x - ox]
                out.extend(raw[start:end])
    projected = bytes(out)
    checked = decode(projected, manifest, target, guard)
    result = Projection(projected, capture.witness, hashlib.sha256(raw).hexdigest(),
                        target, checked.witness)
    guard()
    return result
