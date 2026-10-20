## Concurrent furnishing stop request

Added an offline stop-request command that records the existing permanent
furniture `stop.json` without acquiring the active executor's locks. It binds
the exact original manifest and directory identities while permitting live
index appends. No effect key, receipt, allocation or native command is changed.
An admitted placement may still complete; no native cancellation or quiescence
is claimed. See `docs/FURNITURE_CONCURRENT_STOP.md`.

27 warnings-denied POSIX component/CLI tests pass. They exercise real file and
process behavior with identity-only manifests, not a complete native batch or
live fortress. Original owner integration is source-reviewed only.
