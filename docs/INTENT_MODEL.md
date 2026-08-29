# Intent, Plan, Action, and Obligation Model

## Intent

An intent says what outcome is desired under constraints. It is tied to a source anchor.

```text
Intent {
  id
  anchor
  summary
  terminal_condition
  constraints
  requested_action_skeleton?
  planner_preferences
  budget
}
```

Intent is not authority and does not mutate.

## Plan

A plan is an immutable deterministic compilation product. It includes:

- exact source anchor;
- ordered action DAG;
- affected entity/map/resource/configuration scopes;
- preconditions;
- postconditions;
- dependencies;
- obligations;
- risk and capabilities;
- leases;
- checkpoint requirement;
- predicted semantic diff;
- expiry;
- plan digest.

Any semantic change produces a new digest and plan ID.

## Action

An action is a registered typed mutation. It is not a command string. Registry entries define:

- schema;
- minimum risk;
- scope extraction;
- capability;
- pre/postcondition generation;
- immediate versus temporal behavior;
- idempotency;
- bridge mapping;
- compensation;
- reconciliation;
- compatibility and tests.

## Action state

```text
prepared
→ committing
→ applied_awaiting_verification
→ verified

prepared/committing/applied_awaiting_verification
→ cancel_requested
→ cancelled

verified/cancel_requested
→ compensation_pending
→ compensated

committing/applied_awaiting_verification
→ indeterminate

nonterminal → failed
```

A receipt cannot jump directly from committing to verified without normalized postcondition
evidence.

## Obligation

An obligation owns temporal completion.

```text
Obligation {
  id
  action_id
  terminal_predicate
  failure_predicate?
  deadline_game_tick
  poll_interval
  stable_observation_count
  blockers
  budget
  cancellation_strategy
  evidence
  state
}
```

Mining, construction, hauling, training, and production are typical obligation-producing work.

## Prepare and commit

Preparation:

- load exact plan;
- verify authority and expiry;
- acquire/reserve leases;
- refresh affected state;
- re-evaluate constraints;
- checkpoint if required;
- obtain bridge prepare token;
- persist evidence.

Commit:

- persist dispatch intent/idempotency;
- send typed bridge request;
- persist receipt;
- observe target state;
- verify or register obligation.

## Idempotency

Same idempotency key and same content resumes or returns the prior result. Same key with different
content is a conflict. Possible dispatch plus missing receipt is `indeterminate`, not safe retry.

## Compensation

Compensation is a new action, not time travel. It needs authority, preconditions, risk
classification, effect evidence, and postconditions. It may fail or be impossible.

## Cancellation

```text
request
→ prevent future dispatch
→ ask effect to stop
→ drain in-flight work
→ observe/reconcile
→ compensate if authorized
→ finalize terminal state
```

Dropping an MCP call never deletes the obligation.

## Static versus recipe planning

Phase zero implements a static planner over requested semantic actions. Recipe planning later maps
higher-level objectives to the same plan contract. Optional model-generated plans remain
untrusted drafts until deterministic validation and sealing.
