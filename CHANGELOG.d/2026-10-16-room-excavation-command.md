## Executable single-capture room layout to excavation

Added scripts/survey_room_excavation.py using the existing isolated map reader
and authority. One padded source read is closed before complete layout/terrain
compilation; the command exports the legacy original-room excavation handoff
without a second read or automatic game mutation. Saved compilations revalidate
and export offline, with no credentials or source contact. Nine bounded exports
include exact residual work, complete room/furniture intentions, a restored
legacy terrain survey, full replay evidence and a compact Agent Turn report.

20 CLI/TCP methods passed alongside the earlier 25 component methods. Native
responses are synthetic, not DFHack/live-game qualification. Source closure,
authority revocation, missing/empty/inconclusive work, lost replies, original
identities and one-shot output behavior are covered. Existing native methods,
dig review/confirmation/idempotency and admission boundaries are unchanged.

See docs/ROOM_LAYOUT_EXCAVATION_COMMAND.md.
