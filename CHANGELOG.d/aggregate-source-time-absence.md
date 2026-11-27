# Source-time absence in aggregate queries

Single-field and multi-field aggregate groups and numeric exclusions now use
the same presence classification as their row filter. An explicit absence observed
after the selected tick is reported as `future`. It becomes `absent` at its
observation tick, including when querying retained snapshots.
