# Route-aware furniture assignment — Python development checked

The new pure allocator combines complete map/1.5 decoding and bounded candidate
routes with the existing global exact-item assignment solver. All original
constraints survive. Unknown target terrain is not a zero supply claim; shortages
never produce partial executable plans. Sixteen actual Python tests pass.

This does not yet add a native acquisition command or Rust/MCP route. Source
facts remain caller supplied. No compatibility, protocol, production registry,
mutation authority or live admission changes. Broader beads
`df-franken-storage-mvcc-54h.4` and `df-dfhack-bridge-plane-c-pic.3/.4` stay open.
