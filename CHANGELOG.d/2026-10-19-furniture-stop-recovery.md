## Recover original furnishing effects behind incomplete stop markers

Existing inspect/query/cancel can now recover exact original history when a
concurrent stop writer leaves empty or malformed bounded stop bytes. The marker
stays pinned and untouched, new placement remains fenced, and invalid original
manifest/index/receipt custody still refuses. CLI recovery holds original custody
through final output without another query. See `docs/FURNITURE_STOP_RECOVERY.md`.

57 warnings-denied Python tests pass, including real journals/TCP/CLI and killing
the actual stop writer during a lost placement reply. Native DFHack, Rust,
room/terrain-backed recovery and full-repository qualification are not claimed.
