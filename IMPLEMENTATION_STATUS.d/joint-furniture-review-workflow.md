# Joint-delivery review integration

Source and executed Python workflow tests now cover explicit alternative-item
search through review_route_furniture.py create. The saved review freezes its
new selections; check cannot search, reorder or substitute missing items.
No extra native acquisition, permission or journal mutation is introduced.

62 tests pass, including nine new runtime/CLI cases with explicit acquisition,
budget and packet doubles. Pinned default and sequence-only fixture bytes remain
identical. Five weakened core implementations fail assertions. This is not native
TCP, private batch-parent, Rust/MCP, real DFHack or full-workspace qualification.
See docs/JOINT_FURNITURE_DELIVERY.md. Broader beads remain open.
