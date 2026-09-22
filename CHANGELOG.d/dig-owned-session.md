### Own mining connections across separate foreground stages

Add DigSession above the existing Rust dig coordinator. Initial observation,
preparation and commit share the original native connection; commit has no
connection factory and cannot reprepare or recover dispatch permission from
journal bytes. Lost replies recover only through explicit query/cancellation.
Retained terminal and permanent-Unknown records skip native connection creation.

Mode, source/scope, current authority, monotonic source floors and aggregate work
reservations are checked before native work. Acquisition failures clear cached
selection, and new observations abandon prior ephemeral permission without
forgetting the durable obligation. Mandatory runtime guard hooks remain explicit.
Sixteen Rust regression tests are registered, including actual RPC/session/journal
composition over a fragmented wire; all are uncompiled and unexecuted here.
No native wire, dependency or production-admission change. See docs/DIG_SESSION.md.
