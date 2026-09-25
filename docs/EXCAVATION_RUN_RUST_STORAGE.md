# Concrete Rust excavation storage and foreground workflow

The existing 1.18 Rust coordinator now has a concrete Linux storage backend and
foreground entry points joining it to the native TCP client. No Python subprocess,
new native method, new MCP tool, dependency or production runner is introduced.
Beads: df-dfhack-bridge-plane-c-pic.4/.5 and df-action-coordinator-exec-ero.4.
This supersedes the missing concrete storage/source statements in
EXCAVATION_RUN_RUST.md; those earlier evidence limits remain historical.

## One persistent owner directory

The trusted runtime chooses one dedicated absolute directory. It must already
exist, contain no symlink component, and have exact mode 0700 with root/effective
user ownership. The backend uses exactly `excavation-run-v1_18.journal`. The
create API requires an empty directory and exclusive creation. Existing empty,
partial or corrupt journals are never treated as new work. Recovery and offline
inspection require the existing nonempty journal. Any additional directory entry
is refused; a new filename cannot bypass that directory's pending-work fence.

The supported implementation is Linux x86_64/aarch64 with a trusted
`/proc/self/fd` mount. Other platforms fail closed. Directory traversal opens each
real path component with O_NOFOLLOW|O_DIRECTORY. Descriptor-relative opens use
only the kernel's pinned fd entry, not a caller-controlled symlink. File opens
are nonblocking so special files can be classified without waiting for a peer.

Exclusive nonblocking locks on both directory and journal remain owned by the
coordinator. Every I/O checks the named directory against the pinned descriptor,
directory membership, file identity/length/change stamps, exact private modes,
and single-link regular-file custody. Writes are append-only, bounded to 2 MiB;
truncate, overwrite, repair and replacement are absent. A failed write, sync or
custody check fences the backend. Full content/chain checks remain the existing
coordinator's responsibility and are repeated at its publication boundaries.

Every publication syncs the file AND the pinned directory before the coordinator
can acknowledge it or construct a dispatch permit. File sync success alone is
insufficient. Errors preserve complete or partial files for investigation. A
read-only archive opens without write permission and never calls write, flush,
sync or truncate. It returns immutable historical entries, not an object with
start/cancel methods. Empty archive results cover this journal only.

Locks are host advisory coordination, not a global game-controller lease or an
external anti-rollback root. A trusted owner replacing/removing the entire store,
an external UI/plugin, unqualified network filesystems and forced host death are
not excluded by this implementation. Do not change configured directories to
bypass unresolved history. Filesystem syscalls have no hard interruption bound.

## Public Rust APIs

`private_file::create_private_excavation(directory, binding, context)` returns a
new file-backed coordinator. `open_private_excavation(directory, fortress,
context)` strictly reopens the existing one. `inspect_private_excavation(...)`
requires Query authority only and no native credentials; it returns the immutable
`ExcavationArchive`. The raw file constructor is not public.

`workflow::initialize(directory, binding, context)` explicitly creates a new
store without contacting native code. Obtain the binding from a validated control
observation; a stored binding itself confers no authority. Initialization and
subsequent starts are deliberately separate: the execution path never silently
creates a replacement store.

`workflow::start(directory, ConfirmedExcavationStart { plan, confirmed_digest,
nonce }, context, cancellation)` opens and validates storage before connecting.
It rejects unconfirmed plans, old keys and pending work, negotiates one concrete
control connection, and invokes the actual durable coordinator. The same
non-cloneable dispatch permit and one-use preparation govern native commit.

`workflow::recover(directory, ExcavationRecovery { fortress, key, action, nonce },
context, cancellation)` opens the existing store first. Cached terminal history
returns without reading the operator environment or opening a socket. Otherwise
it uses one recovery-only connection and performs exactly one Query or Cancel.
No terrain read is required. SourceLost remains terminal historical evidence but
unresolved operator work. There is no resume-commit operation or internal poller.

The foreground functions carry one shrinking cooperative deadline and byte
allowance through journal replay, connection negotiation and coordination. They
charge the journal's actual retained bytes and the fixed negotiation allowance;
the source and coordinator enforce their own remaining sub-budgets. The supervising
runtime supplies a fresh nonce, current scoped OperationContext and cancellation
signal for each operation. Cancellation does not terminate the native stop owner.
This is not yet an Asupersync Cx/global-clock-lease or MCP session integration.

## Added tests and executed limits

    cargo test --locked --offline -p dfmcp-adapter excavation_run
    python3 scripts/check_excavation_rust_rpc_vectors.py
    python3 scripts/probe_excavation_linux_custody.py

There are 17 new Linux storage/workflow Rust tests, plus one joined test using the
actual private backend, coordinator and actual TCP client against a fragmented
protocol double. They cover disk reopen after lost commit, strict offline replay,
locking, existing empty/partial files, mode/link changes, directory/file replacement,
same-size rewrites, append-only refusal, cancellation, pending-work fences, and
custody failure after preparation before commit. Together with the 16 transport
tests, this work adds 34 Rust tests. ALL remain UNCOMPILED AND UNEXECUTED here:
Cargo, rustc, rustfmt and the locked dependency cache are unavailable.

Executed: six independently constructed Python request vectors, and eight Python
experiments exercising the Linux primitives used by the design. The latter check
flag values, proc-fd identity, cross-process directory and separate-open file
locking, file+directory sync/readback, nofollow/link metadata, nonblocking FIFO
classification, and descriptor pinning under directory replacement. Those
experiments DO NOT execute the Rust implementation, prove it compiles, simulate
power loss, or qualify durability. Their temporary evidence files are retained.

Rust compilation, formatting/Clippy, actual Rust test execution, real DFHack SDK
and protobuf integration, live-fortress campaigns, physical failure qualification,
full-repository checks and production admission remain outstanding. Native/Python
wire contracts, compatibility registry, dependencies and production map are unchanged.
