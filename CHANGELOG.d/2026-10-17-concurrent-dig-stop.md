## Concurrent original dig-batch stop requests

Add `scripts/request_dig_blueprint_stop.py` to durably revoke an original batch
while a foreground executor holds its directory/manifest locks. It publishes
only the existing immutable stop record, checks exact original manifest/effects
identity, and never contacts native code or changes original effect custody.
Partial publications fence without repair; exact existing markers can be
re-synchronized after lost acknowledgments. In-flight effects may complete and
remain subject to original-key recovery. No native pause or quiescence claim.

29 actual private-file/process Python tests pass; native execution, room semantic
replay, Rust, live-game, and full-repository qualification are not claimed.
