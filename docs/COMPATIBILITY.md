# Compatibility

## Compatibility tuple

Support is never stated as “works with DFHack” in the abstract. It is attached to:

```text
DF version/build
DFHack version/commit
bridge version
canonical/bridge schema versions
platform
loaded mod fingerprints or mod policy
action family
semantic probe set
```

## Levels

### Exact

Certified tuple and all required probes pass.

### Compatible

A registered translation applies and probes pass.

### Degraded read-only

A safe observation subset is available; mutations are disabled by family.

### Unknown

The tuple is not certified. Doctor and bounded raw diagnostics may run; no affected mutation.

### Incompatible

Required identity, state, or bridge semantics fail. Session is doctor-only or refused.

## Per-family support

Compatibility is granular. Example:

| Family | Status | Reason |
|---|---|---|
| identity/tick/pause read | exact | fixture and probes pass |
| unit summary | compatible | one translated enum |
| labor write | exact | round-trip probe passes |
| work orders | degraded | condition schema changed |
| military | disabled | not certified |
| checkpoint | unknown | platform save coordination untested |

## Manifest

A compatibility manifest contains:

- tuple;
- supported read groups/fields;
- supported action kinds;
- translations;
- consistency/freshness;
- native ID/generation behavior;
- journal/idempotency;
- limits;
- probe results;
- fixture digests;
- known failures;
- certification evidence;
- expiry/review policy.

## Probe philosophy

Version numbers are necessary but not sufficient. Probes test meaning:

- fortress identity stable across read/reload;
- pause write reads back;
- unit IDs/generations;
- coordinate orientation;
- designation round trip;
- work-order condition round trip;
- event deduplication;
- save/checkpoint visibility.

Mutation probes use disposable fixtures only.

## Unknown fields and enums

- preserve unknown optional wire fields when codec supports it;
- reject unknown required semantic fields;
- retain raw code in diagnostics;
- do not map unknown enum to a plausible known value;
- disable affected derivations/actions;
- mark canonical facts unsupported/unknown.

## Upgrades

On tuple change:

1. drain mutation;
2. re-handshake;
3. find candidate manifest;
4. run probes;
5. create new epoch when canonical interpretation may change;
6. rebuild derived indexes;
7. re-enable each action family only after status known.

## Mods

Mod handling modes:

- `none-certified`;
- explicit allowlist with fingerprint/schema extension;
- observation-only unknown mods;
- refuse unknown gameplay-semantic mods.

A mod can add typed extension schemas but cannot inject executable bridge functions through data.

## Support claim format

A release note should say:

> Certified for DF X, DFHack Y commit Z, bridge B, platform P, no gameplay-semantic mods, with
> read groups R and action families A. Work-order mutation is degraded/disabled for tuple T.

Anything less precise is marketing, not compatibility.
