# Proposed DFHack bridge

This directory is a design and compile scaffold. It is **not a DFHack plugin or daemon yet**.

The C++ target does not include DFHack headers, use DFHack plugin registration macros, link
DFHack, open an IPC socket, decode semantic requests, or call the Lua helper. Its initialization
entry point deliberately fails so that copying the standalone `.so` into a DFHack installation
cannot manufacture a successful bridge.

The checked-in pieces are:

- `include/dfmcp_ipc.h`: experimental length/type/CRC framing code;
- `src/dfmcp_bridge.cpp`: fail-closed compile placeholder;
- `lua/dfmcp_helpers.lua`: unconnected research helpers, not an authorized MCP surface;
- `CMakeLists.txt`: standalone compile check, not a DFHack installation target.

For a syntax/build smoke check only:

```bash
cmake -S bridge/dfhack-plugin -B build/dfmcp-bridge-smoke
cmake --build build/dfmcp-bridge-smoke
```

Do not copy the result into a DFHack `plugins/` directory. The next acceptance milestone is an
actual read-only DFHack plugin handshake and observation capsule, as described in
[`../../IMPLEMENTATION_STATUS.md`](../../IMPLEMENTATION_STATUS.md).
