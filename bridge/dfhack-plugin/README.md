# Authenticated read-only DFHack bridge

This directory contains the real protocol-1.0 plugin. Its CMake target uses
DFHack's `dfhack_plugin` macro, generated protobuf messages, and supported native
DFHack APIs. It exposes exactly `Handshake` and `ReadObservation`, with bounded
loopback authentication, nonce/version checks, strict citizen ordering and
pagination. Names are converted from DF encoding before UTF-8 bounding.

Build it inside one exact matching DFHack checkout with
`scripts/qualify_dfhack_plugin.sh`; a standalone custom socket target is not the
bridge. The obsolete experimental CRC/socket header has been removed. Unconnected
Lua research helpers do not form an MCP command surface.

Protocol 1.1 is a separately built development generation with retained
announcement fields; it does not extend protocol 1.0 in place. See
[the live read contract](../../docs/LIVE_DFHACK_READ_PATH.md) and
[implementation status](../../IMPLEMENTATION_STATUS.md) for exact evidence limits.
A compiled plugin or successful development read grants no registry admission,
production runner admission, or mutation authority.
