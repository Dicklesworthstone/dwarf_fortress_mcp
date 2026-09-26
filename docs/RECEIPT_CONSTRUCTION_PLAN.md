# Receipt-linked furniture construction plans

`scripts/track_construction_plan.py` monitors a bounded set of original
furniture/1.19 `Placed` receipts as one construction goal. It establishes whether
every selected bed, chair and table meets its receipt-linked condition in the
same complete operations/1.4 capture, and whether that shared condition survives
the requested sequence of sampled game ticks. This supports a whole room or
furniture set without combining independently timed completion claims.

The implementation is an explicitly unadmitted Python development workflow.
It uses the existing query-only native bindings and preserves the original
placement journals. Its machine contract is
`architecture/construction_plan_monitor_v1.json`; the single-receipt condition
and custody foundations are documented in `RECEIPT_CONSTRUCTION.md`.

## Original receipts define the plan

A plan contains 1..32 complete canonical furniture/1.19 `Placed` records.
Prepared, cancelled or indeterminate placement records are ineligible. The
placement keys, native building IDs, original construction job IDs, selected item
IDs and target positions must be unique. All receipts must name the same
furniture-plugin generation, fortress folder, site and map dimensions. Input
order does not affect the goal: canonical order is ascending native building ID.
Encoded goals and samples must already use that order.

The receipt set is an explicit monitoring selection. It does not authenticate a
previously requested blueprint or action DAG, prove that every originally
requested placement is represented, or import a high-level furniture plan
automatically. A caller must choose the complete intended receipt set before
starting; the monitor proves its condition only for that selected set.

Each member retains the exact original placement bytes and inherits policy
`dfmcp.receipt-construction-condition/1`. It requires the original building type,
one-tile footprint and maximum build stage, the original singleton item with its
type/subtype/material identity, complete construction stage, no held construction
or removal job, and the original item installed in that building with no container
or observed job attachment. Building or job disappearance does not establish
completion. Native identity, clock, ID-horizon and construction-stage regressions
remain explicit invalidations; a removal job remains a failure.

The plan policy is `dfmcp.receipt-construction-plan-condition/1`. Its digest
binds the sorted receipt set and one fixed deadline, cadence, stability count,
minimum span, maximum gap and total observation allowance. Sampling or reopening
the journal cannot add or substitute a receipt, retarget furniture, change the
endpoint, extend a deadline or renew the original observation allowance.

## One capture for the entire set

Each fresh acquisition uses one foreground TCP owner and this fixed sequence:

1. Bind furniture `Handshake`/`QueryPlacement` and operations
   `Handshake`/`ReadObservation`; verify both native manifests.
2. Query every original placement receipt in canonical plan order.
3. Acquire every page of one complete operations capture, verify its SHA-256,
   and verify its release acknowledgment.
4. Query every original placement receipt again in canonical plan order.
5. Verify the complete sample and evaluate every target against the shared
   decoded operations capture.

Every receipt must remain byte-for-byte equal to its original before and after
the capture. Each query's native retained-record count must cover the entire
selected set; unrelated retained work may increase that count between queries.
Furniture generations must equal the original plan generation;
operations generation is independently pinned and is never numerically equated
with furniture generation. Both plugin families must report matching software
versions. Missing receipts, source drift, partial or mixed pages, malformed wire
values, missing release acknowledgment and a lost trailing receipt query cannot
publish a successful sample. There is no reconnect or automatic retry within an
acquisition.

Serialized evidence records these checks and their inputs. It is not a signature
or independent proof that network I/O occurred on one connection. Canonical world
anchors remain null: native IDs and a complete native capture do not create a
canonical world generation.

## Stability belongs to the whole plan

The monitor keeps one global stability streak. A sample contributes only when
every target's condition holds in that same capture. A bed completing in one
capture and a table completing in another do not establish simultaneous success
if the bed no longer satisfies its condition in the second capture.

Distinct advancing game ticks must satisfy the fixed cadence, minimum span and
sample count. Repeated paused captures do not add stability samples. Any false
or unknown target resets the entire streak; per-target identity and stage
regressions are still retained. A long observation gap, interrupted acquisition
or changed capture at the same tick resets stability. Reopening after an
interrupted read requires a new explicitly started read and cannot recover the
prior process's publication permission.

Terminal evidence is immutable. Even `satisfied` describes a historical sampled
condition. It does not prove continuous monitoring, current building usability,
terrain safety, room assignment, mining/building causality or a verified game
checkpoint. It does not discharge original placement effects or permit retry of
an uncertain effect. Cancellation affects this monitor only.

## Import and run

The input is a private JSON file with exactly this schema and shape:

```json
{
  "schema": "dfmcp.construction-plan-receipts/1",
  "receipts": [
    {"canonical_record_hex": "<complete first DFMBR019 record in hexadecimal>"},
    {"canonical_record_hex": "<complete second DFMBR019 record in hexadecimal>"}
  ]
}
```

Replace each placeholder with the unchanged `canonical_record_hex` from an
eligible furniture placement result. The bundle must contain 1..32 records,
remain within 512 KiB and be an owned regular single-link exact-mode `0600`
file under a real owned exact-mode `0700` directory. Duplicate JSON keys and
unknown fields are refused. The input is read only; the monitor stores its own
copy of the original receipts in its new journal.

Configure the isolated client process with exactly these four `DFMCP_*`
variables:

| Variable | Value |
|---|---|
| `DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR` | Exactly `1`. |
| `DFMCP_CONSTRUCTION_MONITOR_ENDPOINT` | Canonical numeric IPv4 loopback endpoint, such as `127.0.0.1:5000`. |
| `DFMCP_BUILD_TOKEN` | Existing furniture plugin query credential. |
| `DFMCP_OPERATIONS_PAGED_TOKEN` | Existing operations plugin read credential. |

Every other `DFMCP_*` variable is refused, including placement and production
admission settings. The native game process retains its existing plugin opt-ins
and credentials. The monitor does not change native configuration.

Select a future absolute deadline from observed game ticks. The deadline below
is illustrative and must be replaced for the actual fortress:

```sh
python3 scripts/track_construction_plan.py start \
  --journal /private/construction/bedroom.plan-monitor \
  --receipts-file /private/construction/bedroom.receipts.json \
  --deadline-tick 900000 --stable-samples 2 --stable-span-ticks 10 \
  --interval-ticks 10 --max-gap-ticks 1200
python3 scripts/track_construction_plan.py sample \
  --journal /private/construction/bedroom.plan-monitor
python3 scripts/track_construction_plan.py inspect \
  --journal /private/construction/bedroom.plan-monitor
python3 scripts/track_construction_plan.py cancel \
  --journal /private/construction/bedroom.plan-monitor
```

`start` and each nonterminal `sample` perform one bounded foreground acquisition.
The operator explicitly starts each sample; the workflow neither advances game
time nor starts a polling worker. `inspect` requires no bridge or credentials.
Terminal `sample`, `inspect` and `cancel` return retained evidence without native
contact or file writes. Cancelling an active monitor records only its cancellation.

Every response is one complete JSON object, bounded at 64 KiB, with schema
`dfmcp.construction-plan-monitor-result/1`. It includes the goal identity,
per-target evidence, shared progress, exact receipt/capture references, custody
status and an Agent Turn showing unresolved monitoring work. Errors preserve
uncertainty and do not expose credentials or arbitrary operator arguments.

## Journal and resource bounds

The separate `DFMPJR01` journal uses checksum domain
`dfmcp.construction-plan-journal/1` followed by NUL. The frame header remains a
big-endian body length, sequence and previous-frame checksum. Typed frames contain
the initial goal/endpoint, read start, complete shared sample or cancellation.
Replay verifies full canonical evidence and derives transitions again; a saved
phase label cannot substitute for the evidence that produced it.

The owner validates the proposed transition and reserves the complete result
before appending it, synchronizes the file and parent directory, verifies all
old and new bytes, and only then publishes the in-memory result and acknowledges
it. A synchronized read-start precedes native contact. Before creating it, the
owner reserves room for one maximum-sized sample and future cancellation.
Failed acquisition, rendering or publication preserves the unknown read intent.

Custody uses canonical absolute paths, no-follow traversal through every path
component, exact owned `0700`/`0600` modes, single-link regular files and exclusive
nonblocking local locks. It refuses same-size substitution, replaced pathnames,
incomplete frames and corrupt history. It never truncates, repairs, compacts or
evicts evidence. File synchronization is cooperatively deadline-checked; it is
not a hard real-time cancellation guarantee. Local locks and checksums are not
distributed fencing, signatures, anti-rollback authority or protection against
a malicious owner.

| Bound | Maximum |
|---|---:|
| Selected receipts | 32 |
| Canonical goal bytes | 196,800 |
| Canonical sample bytes | 17,174,656 |
| Complete operations capture | 16 MiB |
| Jobs / buildings | 4,096 each |
| Items / job-item attachments | 65,536 each |
| Capture pages | 256, each at most 64 KiB |
| RPC calls for one operation | 327 |
| Connection / notification bytes | 20 MiB / 2 MiB |
| Cooperative work steps | 20,000,000 |
| Custody bytes read/written per operation | 1 GiB |
| Whole-operation wall deadline | 1..60,000 ms |
| Journal size / frames | 128 MiB / 1,030 |
| Goal observations | 512 |
| Required stability samples | 2..64 |
| Complete response | 64 KiB |

One shrinking allowance covers replay, acquisition, validation and publication.
Neither each page nor each target receives a fresh time, I/O or work allowance.
The goal may select fewer observations; journal or work capacity can stop
acquisition sooner while retaining prior evidence.

## Validation scope

Focused regression entrypoints are:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_construction_plan.py --mutations
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_construction_plan test_construction_plan_rpc \
  test_construction_plan_store test_track_construction_plan -v
```

The aggregate checker passed **108 actual Python test functions**: 59 whole-plan
tests (17 core, 14 TCP, 15 private-store and 13 CLI) and all 49 existing
single-receipt tests. It also rejected **five weakened whole-plan
implementations** through regression assertions. The scope includes canonical
multi-receipt identities, shared-capture evaluation and stability, all receipt
boundaries, foreground TCP acquisition, durable restart and interruption
handling, private custody, bounded whole results and unchanged placement evidence.

The exact result and all **20 input source hashes**, rechecked unchanged after
execution, are retained in
[`docs/evidence/construction-plan-monitor.json`](evidence/construction-plan-monitor.json).
This is executed Python/TCP/POSIX development evidence. It does not establish a
real DFHack SDK build, live fortress, Rust/MCP integration, physical power-loss
safety, full qualification or production admission.

Owning beads are `df-dfhack-bridge-plane-c-pic.4` and
`df-dfhack-bridge-plane-c-pic.5`. This implements bounded subsequent-observation
and recovery behavior within their broader unfinished scope; both remain open.
