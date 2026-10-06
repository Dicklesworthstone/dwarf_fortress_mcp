# Original room request to exact furnishings and sampled construction completion

The Python development workflow now carries a complete original RoomPlan through
native inventory allocation, durable one-attempt furniture placement, recovery
and whole-set construction monitoring. There is no need to manually reconstruct
slots or extract a smaller successful subset between these stages.

This is not an admitted MCP runner or automatic room-building controller. It
preserves room intent, not the earlier terrain monitor's completion history.
Excavation, fresh terrain review and native room assignment remain separate.

## Compile the original intention offline

Keep the existing canonical room request and compile it without native access:

```sh
python3 scripts/plan_rooms.py compile --request-file room-request.json \
  > original-room-plan.json
```

Bedroom-cluster and dining-hall geometry, every furnishing slot, dependency,
material/subtype/distance constraint and excluded item/region remain in the plan.
Compilation observes neither terrain nor furniture. Check each command's exit
status before treating redirected bytes as a valid artifact.

## Allocate once and export the complete room handoff

Use only the isolated inventory-read environment: configure
`DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION=1`,
`DFMCP_FURNITURE_ALLOCATION_ENDPOINT` and `DFMCP_OPERATIONS_PAGED_TOKEN`.
The endpoint is the existing numeric IPv4 loopback source. Do not mix unrelated
DFMCP profile variables or place credentials in command arguments.

```sh
python3 scripts/plan_rooms.py allocate --plan-file original-room-plan.json \
  --emit room-handoff > room-furniture-handoff.json
```

`--request-file room-request.json` is an alternative to the compiled plan input.
Default `--emit report` retains the original diagnostic allocation response.
On successful capsule export, stdout is exact canonical
`dfmcp.room-furniture-handoff/1` JSON without a trailing newline. A shortage or
failure returns exit status 2 and a refusal object, never a partial handoff.
The report mode still exposes complete shortage diagnostics without a plan.

The command acquires all retained operations pages, verifies the whole capture
and release, and closes the socket before local allocation and encoding. Every
helper shares the same foreground budget and live query-authority checks. Even
narrow export reserves the complete diagnostic report before publishing bytes.
A broken or short stdout write is not retried and produces no second JSON object.

The capsule binds full room geometry and original constraints to exact selected
items and their historical inventory source. Two room intentions with identical
furniture targets but different walls or corridors have different identities.
It reserves no items, creates no preparation and grants no placement authority.
Exported hashes are not signatures or independent acquisition attestations.

## Place using the existing reviewed one-attempt protocol

Switch to the isolated furniture/1.19 environment, removing the inventory-profile
variables. Configure the existing furniture opt-in, endpoint and credential.
Create an empty owned private directory and import the capsule:

```sh
mkdir -m 700 "$HOME/room-furnishings"
python3 scripts/furniture_batch.py init --directory "$HOME/room-furnishings" \
  --room-handoff room-furniture-handoff.json
```

The handoff supplies the original fortress; selector overrides are refused.
Initialization checks a native observation but performs no placement. Keep the
returned batch ID. Each next step requires fresh review and explicit confirmation:

```sh
python3 scripts/furniture_batch.py review --directory "$HOME/room-furnishings" \
  --batch-id "$BATCH_ID"

python3 scripts/furniture_batch.py advance --directory "$HOME/room-furnishings" \
  --batch-id "$BATCH_ID" --expected-plan "$EXPECTED_PLAN" \
  --confirm-review "$REVIEW_SEAL"
```

The existing separate placement opt-in is required for advance. One advance can
attempt only one original placement. Lost replies leave original-key recovery
work; use the existing query/cancel commands rather than repeating advance or
creating a replacement batch. Reopening never reallocates or changes the plan.
Every room-backed key binds the complete batch definition, including room-only
geometry. Old exact-plan and standalone-handoff keys are unchanged.

## Monitor every original furnishing together

After all original steps have registered Placed receipts, switch to the separate
query-only construction-monitor environment and use the existing completion CLI.
Keep the completion journal outside the original batch in an owned 0700 directory:

```sh
python3 scripts/track_furniture_batch.py start \
  --batch "$HOME/room-furnishings" --batch-id "$BATCH_ID" \
  --journal "$HOME/room-monitor/completion" --deadline-tick "$ABSOLUTE_DEADLINE"

python3 scripts/track_furniture_batch.py sample \
  --journal "$HOME/room-monitor/completion"
```

The deadline is a fixed absolute game tick with enough time for the stability
policy. Every original receipt is checked before and after the same complete
operations capture on one connection. Every target must meet its condition in
the same advancing-tick stability window; prior per-building success is not
latched. See [room-backed completion](ROOM_FURNITURE_COMPLETION.md) for environment,
inspection, cancellation after original-file loss and recovery details.

`requested_room_plan` and every target's room area/unit remain visible.
`complete_original_room_furnishings_sampled_condition=true` means the historical
shared furnishing condition passed with verified original batch custody. It is
not whole-room completion: `room_completion_proven`, `terrain_completion_proven`
and `room_assignments_observed` remain false. No monitor discharges or retries
placement effects, certifies current usability or claims continuous stability.

## Executed development tests

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts:tests python3 -m unittest \
  test_room_furniture_handoff test_room_furniture_batch \
  test_room_furniture_completion test_room_allocation_pipeline -v
```

All 55 new methods passed in final split runs with no skips. The full maximum
pipeline retained 32 slots and 646 exclusions, decoded multipage inventory with
2,000 extra items, executed all 32 synthetic native placements, then reached
whole-set construction satisfaction. Original input files were deleted after
batch creation to exercise retained-custody continuity. Source drift, shortages,
lost replies, forged subsets, torn/missing custody, authority revocation, bounds
and short output failures are covered. Sixty-five additional legacy comparisons
matched prior canonical outputs and completion representations byte-for-byte.

These tests use actual Python clients, CLI subprocesses and private files with
joined synthetic TCP peers. Operations bytes are independently assembled;
placement receipts use production codecs. They are not DFHack SDK/ABI/live-game,
Rust/MCP or full-repository qualification. Production admission is unchanged.
