# DFHack Native MCP Bridge Plugin

This directory contains the out-of-process native DFHack bridge daemon (`dfmcp_bridge`) connecting Dwarf Fortress and DFHack to the Rust semantic control plane (`dfmcp-adapter`).

## Architecture

- **Trust Isolation**: Dwarf Fortress and DFHack run in an isolated process to strictly preserve memory safety (`unsafe_code = forbid`) and eliminate C/C++ FFI hazards in the Rust trust domain (`INV-001`).
- **Binary IPC**: Communicates over Unix Domain Sockets (`/tmp/dfhack-mcp.sock`) using 10-byte length/type/CRC32 framed binary messages.
- **Game Thread Safety**: All DF state queries and mutations execute synchronized on the game thread during DFHack's `onUpdate` tick callback.

## File Map

- `CMakeLists.txt`: Build configuration for compiling the shared library plugin.
- `include/dfmcp_ipc.h`: Big-endian binary framing and CRC-32 checksum codec.
- `src/dfmcp_bridge.cpp`: C++ plugin implementation, non-blocking IPC socket listener, and dispatch loop.
- `lua/dfmcp_helpers.lua`: High-level Lua reflection helpers and safe pcall wrappers for fortress inspection.

## Build Instructions

```bash
mkdir build && cd build
cmake -DDFHACK_DIR=/path/to/dfhack ..
make
```
