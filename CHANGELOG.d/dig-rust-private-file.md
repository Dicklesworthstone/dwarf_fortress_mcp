### Concrete private-file backing for the Rust mining coordinator

Add Linux x86_64/aarch64 `open_private_dig`: no-follow pinned-directory opens,
exact private owner/mode checks, single-link regular files, exclusive locking,
append-only writes, file/parent synchronization and repeated inode/extent custody.
Offline opens are read-only through the storage trait; missing, empty or corrupt
recovery files are refused without create, repair, truncate or dispatch permission.
Open and replay share a cooperative deadline and reject unavailable platforms.

Register sixteen file/runner Rust tests, including lifecycle, lost replies, actual
process locking, per-descriptor sync faults, corrupt/torn histories and immutable
offline evidence. All are uncompiled/unexecuted, as are the sixteen coordinator
groups. The independent Python reference passes, not Rust or filesystem execution.
Update root status and changelog without transferring old qualification. No native
wire, dependency, Python recovery, MCP route or production admission is changed.
