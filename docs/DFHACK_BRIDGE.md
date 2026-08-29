# DFHack Bridge Design

## Objective

Provide structured, bounded, versioned access to Dwarf Fortress through supported DFHack
mechanisms while keeping native code and unstable game structures outside the safe-Rust trust
domain.

## Why out of process

- no C/C++ FFI in Rust;
- no ABI coupling;
- bridge restart can be detected;
- payloads can be validated and recorded;
- compatibility logic has an explicit boundary;
- the bridge can use DFHack’s supported execution context.

## Proposed components

```text
dfmcp-bridge client (Rust)
  ↕ authenticated framed protobuf
dfmcp DFHack service/plugin or Lua-backed service
  ↕ DFHack APIs
Dwarf Fortress
```

A bootstrap command may install/start the service, but the production API is not an arbitrary
`dfhack-run` command endpoint.

## Handshake

Required fields:

```text
bridge_protocol_versions
schema_versions
bridge_instance_id
game_instance_id
DF version/build/platform
DFHack version/commit
loaded mod fingerprints where available
read groups and limits
action kinds and limits
field support matrix
consistency classes
operation-journal support and retention
semantic probe results
frame/depth/count/string limits
```

Unknown required versions fail. Known read-only compatibility may remain available when mutation
families are disabled.

## Read transaction

A read request declares field groups, selectors, map regions, event cursor, consistency/freshness,
and bounds. The bridge captures begin/end tick/sequence and reports whether the result met the
requested coherence.

The bridge must distinguish:

- unsupported field;
- absent semantic value;
- omitted by request/budget;
- failed read;
- stale cached value.

## Mutation

The bridge accepts only typed registered messages. Each carries:

- action schema/version;
- prepare token;
- idempotency key;
- expected bridge/game instance;
- expected revisions/tick window;
- fencing tokens;
- bounded typed body;
- mode: prepare or commit.

No client-selected function name, script, command, pointer, or path is accepted.

## Operation journal

The bridge maintains a bounded idempotency journal:

```text
key
request digest
prepare digest
dispatch state
native operation marker
receipt
game instance
first/last sequence
```

On reconnect, the server asks about unresolved keys. If the bridge restarted and lost the
journal, the coordinator marks affected effects for semantic reconciliation.

## Receipts

Receipt states:

```text
rejected_before_effect
queued
executing
applied
partially_applied
failed_after_effect
unknown
```

`applied` still requires later canonical postcondition verification. Queue acceptance is not
application.

## Bounded protocol

Before allocation:

- frame length;
- record count;
- string/bytes length;
- nesting depth;
- map dimensions/coordinates;
- repeated field counts;
- continuation size.

The Rust normalizer validates identity/revision/graph integrity after decoding.

## Development sequence

1. handshake only;
2. health and pause/tick;
3. fortress identity;
4. bounded unit summary;
5. jobs/buildings/orders/resources;
6. events;
7. selected map chunks;
8. prepare-only pause;
9. idempotent pause commit;
10. reversible action families;
11. guarded actions.

## Bridge tests

- exact handshake fixtures;
- malformed and oversized frames;
- unknown enum/required field;
- bridge restart and game-instance change;
- read coherence retry;
- operation-journal duplicate;
- timeout before send, during queue, after effect;
- test-fort postcondition comparison;
- cross-version probes.
