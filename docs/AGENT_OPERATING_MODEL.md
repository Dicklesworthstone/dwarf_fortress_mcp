# Agent Operating Model

This document defines the agent-facing center of gravity of `dwarf_fortress_mcp`. It is normative for every MCP response, observation projection, planner explanation, task update, error, evidence record, and future CLI/NDJSON surface.

The system is not primarily a collection of game integrations. It is a **closed-loop cognitive and control substrate** for an agent that must steward a partially observed civilization for long periods while minimizing tokens, latency, game-time disruption, irreversible mistakes, and epistemic error.

## 1. Constitutional thesis

An agent should never have to reconstruct the protocol state of the fortress from scattered tool outputs. After any successful or failed call, the agent must be able to answer six questions:

1. **What is currently known to be true?**
2. **What changed since the anchor I understood?**
3. **What matters now, and why?**
4. **What can I legally and safely do from here?**
5. **What are the best next moves, including information-gathering moves?**
6. **How certain is each answer, and what evidence would change it?**

Every tool is therefore a view or transition inside one coherent operating loop. The eleven public tools remain the narrow waist; agent ergonomics are achieved by a shared response contract, not by proliferating commands.

The system must optimize for **decision quality per unit of total control cost**:

```text
control cost = model tokens
             + bridge bytes
             + server CPU and memory
             + wall time
             + game ticks consumed
             + attention displaced
             + risk introduced
             + recovery burden created
```

A faster answer that causes an unnecessary fortress-wide dump, a brittle action, or an unprovable assumption is not efficient.

## 2. The driver's-seat test

Before accepting any interface or subsystem, imagine an agent resuming an unfamiliar fortress after a context reset. The interface fails if the agent must:

- ask for a full world dump to discover what is important;
- remember undocumented relationships between tool outputs;
- infer whether a cursor gap, restore, or adapter restart occurred;
- guess whether a fact is observed, derived, inferred, predicted, stale, or unknown;
- inspect raw game structures to learn which actions are currently possible;
- generate speculative actions before learning capability, risk, budget, or confirmation requirements;
- repeatedly ask for progress when nothing meaningful changed;
- interpret transport success as game-state or goal success;
- blindly retry an indeterminate mutation;
- manually reconstruct active plans, actions, obligations, checkpoints, and pending confirmations;
- spend tokens restating stable context that the server already knows;
- trust a recommendation without its evidence, alternatives, cost, uncertainty, and reversibility;
- lose useful lessons because evidence and outcome records are not linked;
- inherit an old agent's assumptions without knowing their anchor and validity domain.

The design passes only when a newly arrived agent can safely orient, act, verify, and leave a compact handoff for its successor.

## 3. One synthetic control loop

All agent work maps to the same loop:

```text
BOOTSTRAP
  negotiate versions, authority, budget, observation profile, and fortress lineage
      ↓
ORIENT
  receive exact anchor, continuity status, compact briefing, changes, attention, active work
      ↓
INSPECT
  acquire only missing information whose expected value exceeds its cost
      ↓
FORMULATE
  express desired predicates, hard constraints, soft utility, horizon, and stop conditions
      ↓
PROPOSE
  compile deterministic candidate plans and expose assumptions, witnesses, predicted effects
      ↓
COMPARE
  evaluate alternatives and counterfactual branches under explicit policy and resource budgets
      ↓
COMMIT
  revalidate knowledge and authority, fence conflicts, checkpoint when required, dispatch once
      ↓
VERIFY
  observe authoritative post-state, prove predicates, discharge obligations, reconcile ambiguity
      ↓
LEARN
  record episode, causal attribution, surprise, reusable lesson candidate, and policy evidence
      ↓
HANDOFF / RESUME
  emit a compact resumable state of understanding, not a prose memory dump
```

A tool may span several stages internally, but it must say which stage the agent is in and what stage should follow. No subsystem may define a competing lifecycle.

## 4. The canonical Agent Turn Packet

Every tool result, including errors, carries an `agent_turn` object conforming to `architecture/agent_turn_contract.json`. The packet is additive: existing tool-specific fields remain, but the agent always gets the same orientation spine.

### 4.1 Identity and continuity

```json
{
  "schema": "dfmcp.agent_turn/1",
  "operation": "fortress.observe",
  "phase": "orient",
  "session_id": "…",
  "request_id": "…",
  "anchor": {"fortress_id":"7","epoch":3,"sequence":18422,"game_tick":9120031,"state_hash":"…"},
  "continuity": {
    "status": "continuous",
    "basis": {"epoch":3,"sequence":18418,"state_hash":"…"},
    "gap": null,
    "reset_reason": null
  }
}
```

Continuity status is one of:

- `bootstrap`: no prior acknowledged anchor exists;
- `continuous`: exact basis-to-target continuity is proved;
- `heartbeat`: target equals basis and no meaningful change passed filters;
- `partial`: the response is complete only for the named coverage and continuation;
- `gap`: the requested basis cannot be resumed safely;
- `reset`: lineage or observation epoch changed;
- `stale`: response is valid at its anchor but fresher state exists;
- `indeterminate`: effect or observation continuity cannot yet be resolved.

The server never silently repairs a gap or presents a mixed-generation answer.

### 4.2 Briefing

The briefing is the minimum sufficient state for a competent next decision:

- pause and game-time state;
- fortress mode and compatibility posture;
- current mission/objective summary when one exists;
- top resource, welfare, security, production, logistics, and infrastructure indicators covered by the observation profile;
- active plans, actions, obligations, checkpoints, leases, and confirmation gates;
- the highest-severity unresolved uncertainty;
- whether mutation is presently admissible.

The briefing contains facts, not narrative filler. Stable values may be referenced by digest or omitted under a continuous delta profile.

### 4.3 Changes

`changes` is a bounded, semantically ordered set of differences from the acknowledged basis. Each item names:

- kind and subject;
- before/after or event semantics;
- salience and causal relationship when known;
- epistemic status;
- evidence references;
- whether it invalidates a plan, witness, recommendation, or objective assumption.

Changes are ordered by protocol criticality, then severity/urgency, then canonical subject identity. Raw arrival order is never allowed to decide agent attention.

### 4.4 Attention

`attention` answers “what matters now?” rather than “what exists?” Each item includes:

```text
attention_id
category
severity
urgency
confidence
subject scope
plain semantic finding
evidence refs
causal contributors
likely consequence if ignored
expiry / review tick
suggested information or control response
```

Attention is a certified derived projection. It can suggest an intent but cannot authorize one. Score components and tie-break policy are inspectable through `fortress.explain`.

### 4.5 Active work

`active_work` is the complete bounded set of session-relevant:

- pending prepared plans;
- committed actions and exact commit states;
- obligations and their progress/stability/deadline;
- cancellation drains;
- indeterminate effects awaiting reconciliation;
- checkpoint or publication work;
- confirmations awaiting an agent or operator.

An agent should never have to remember a handle from an earlier context window to learn that unfinished work exists.

### 4.6 Affordances

An affordance is a currently expressible, authority-compatible semantic action template, not a raw command string. It includes:

- action or intent family;
- scope and parameter schema;
- capability and risk requirement;
- whether current grants satisfy it;
- hard preconditions already known true;
- preconditions still requiring observation;
- checkpoint and confirmation policy;
- estimated action count, bridge cost, observation cost, and game-time horizon;
- reversibility and compensation class;
- predicted effect class and confidence;
- reasons the affordance is disabled or degraded.

Affordances reduce hallucinated actions and wasted planning calls. They are never promises that commit-time revalidation will succeed.

### 4.7 Recommendations

Recommendations are ranked **protocol next steps**, not unconstrained prose. A recommendation may be informational, deliberative, mutating, waiting, reconciling, or operator-directed.

Each recommendation records:

```text
recommendation_id
tool
intent family or query template
reason
expected utility
expected information value
risk
reversibility
estimated token/byte/wall/game-tick cost
prerequisites
invalidating conditions
confidence and evidence
whether confirmation is required
```

Ranking is lexicographically safety-first:

1. resolve continuity, authority, or indeterminate-effect hazards;
2. prevent imminent irreversible loss;
3. satisfy explicit hard objectives and deadlines;
4. acquire high-value missing information;
5. improve expected fortress utility;
6. reduce future control cost and uncertainty;
7. prefer lower risk, lower cost, and greater reversibility when utility is equivalent;
8. apply canonical tie-break policy.

The server may return zero recommendations. It must never invent activity merely to appear helpful.

### 4.8 Uncertainty and coverage

Every response declares what it does **not** establish. Uncertainty items use the epistemic lattice in section 6 and include a resolution path when one is available.

Coverage names:

- included entity kinds, fields, map regions, event classes, graph relations, and time interval;
- completeness status for each domain;
- source and derived generation high-water marks;
- omitted domains and why;
- continuation tokens and their exact anchor;
- budget consumed and remaining.

An empty result without a complete-domain witness cannot prove absence.

### 4.9 Next-step handles

All drill-downs and suggested protocol calls use typed handles or structured templates. The system never places arbitrary shell, Lua, DFHack, or executable text in a next-step field.

## 5. Observation profiles

Profiles are stable contracts, not vague verbosity levels.

### `pulse`

Purpose: cheapest safe control-loop heartbeat.

Contains:

- continuity and anchor;
- only new critical/high attention items;
- state changes that invalidate active work;
- active work state transitions;
- top one to three recommendations;
- unresolved indeterminate effects;
- no unchanged background inventory.

Target: roughly 150–400 output tokens when nothing exceptional happened.

### `briefing`

Purpose: default orientation after a meaningful interval or context reset.

Adds:

- compact fortress health and operational summary;
- top changes by domain;
- bounded affordance set;
- mission/objective status;
- resource and welfare trend indicators;
- important unknowns.

Target: roughly 500–1,500 tokens.

### `tactical`

Purpose: make or supervise a concrete decision.

Adds:

- requested entity/region detail;
- causal neighborhood and dependencies;
- active plan witnesses and blockers;
- candidate options with predicted effects;
- relevant map/logistics topology;
- finer evidence references.

Target: bounded by the negotiated request budget and explicit interest set.

### `forensic`

Purpose: reconcile an indeterminate effect, diagnose a failure, audit a recommendation, or reproduce a decision.

Adds:

- complete evidence chain for the named scope;
- plan/action/obligation state transitions;
- decision and complexity witnesses;
- compatibility and policy epochs;
- source spans and bridge receipts;
- negative evidence and alternative hypotheses.

Forensic is never the default and may require diagnostic capability.

### `custom`

A custom profile is an explicit union of registered projections with fixed bounds. Unknown projection names fail closed.

## 6. Epistemic model

Every fact or recommendation uses one of these states:

- `observed`: normalized directly from a compatible authoritative bridge observation;
- `certified_derived`: deterministically derived from observed facts with complete provenance and a registered derivation;
- `inferred`: supported by evidence but not entailed by authoritative state;
- `predicted`: output of a forward or counterfactual model;
- `assumed`: supplied as a planning premise and not yet verified;
- `stale`: formerly valid but outside the accepted freshness or epoch contract;
- `unknown`: not established;
- `contradicted`: evidence refutes the claim;
- `indeterminate`: evidence cannot distinguish material outcomes.

Only `observed` and eligible `certified_derived` facts may satisfy mutation preconditions. `inferred`, `predicted`, and `assumed` facts can motivate inspection or candidate generation, but must be converted into witnessed facts before effect authority is issued.

Confidence is not a substitute for epistemic class. A 0.99 prediction remains a prediction.

## 7. Value-of-information planning

The system should not reflexively observe more. Before an inspection step, estimate:

```text
VOI = expected reduction in decision loss
      - observation cost
      - delay cost
      - game-time exposure during delay
      - attention displacement
```

An inspection is preferred when it can change the chosen plan, risk class, checkpoint requirement, or confidence enough to justify its cost. If all feasible answers lead to the same safe action, additional observation is waste.

The agent turn packet exposes why an information request is recommended and which decision boundary it could change.

## 8. Mission and objective model

Long-horizon control needs a declarative mission stack above individual actions.

An objective contains:

- stable objective ID and parent mission;
- desired terminal predicates;
- hard invariants and forbidden states;
- soft utility terms and their policy epoch;
- priority and urgency;
- horizon in game ticks or calendar semantics;
- review cadence;
- dependencies and conflicts with other objectives;
- acceptable risk and resource envelopes;
- completion, failure, suspension, and abandonment predicates;
- evidence requirements;
- owner and delegation scope.

Objectives form a typed dependency/conflict graph. The planner may decompose an objective into subgoals and candidate plans, but it cannot silently rewrite the objective. Every decomposition is an inspectable artifact with a digest.

The first executable slice may support only an ephemeral objective attached to `fortress.plan`; the data model must still preserve the distinction between objective, intent, plan, action, obligation, and evidence.

## 9. Candidate sets and counterfactuals

`fortress.plan` should eventually return a bounded candidate set when materially different safe strategies exist. Each candidate is evaluated against the same anchor, objective, constraints, policy epoch, and resource budget.

Candidate comparison includes:

- predicted terminal predicate satisfaction;
- risk and reversibility;
- expected resource and game-time cost;
- opportunity cost and blocked work;
- robustness to uncertainty;
- checkpoint/compensation burden;
- required observations;
- witness breadth and conflict probability;
- decision-path and tie-break record.

Counterfactual branches are immutable, structurally shared derived worlds. They are explicitly predicted, never confused with canonical state. A branch can propose an intent but cannot dispatch an effect.

## 10. Execution, verification, and surprise

After commit, the agent packet distinguishes:

- dispatch accepted;
- effect observed;
- postconditions satisfied;
- obligation stable-complete;
- objective advanced or completed.

Verification compares predicted and observed deltas. A **surprise record** is emitted when:

- an expected effect is absent;
- an unexpected material effect appears;
- completion takes materially longer or consumes more resources;
- a precondition changes between plan and commit;
- compensation does not restore the predicted state;
- a recommendation ranking would have changed with newly observed facts.

Surprise is the basic unit of useful learning. Silent prediction error prevents accretion.

## 11. Agent-accretive memory

The system accumulates capability by linking evidence, not by appending untrusted prose.

### 11.1 Memory strata

- **Episodic:** exact objective, anchor, plan, action, outcome, evidence, surprise, and cost for one episode.
- **Semantic:** stable fortress or game knowledge supported across episodes and version scopes.
- **Procedural:** reusable intent/decomposition templates with applicability predicates.
- **Policy:** ranking or budgeting parameters with training evidence, epoch, clamps, and rollback path.
- **Negative:** failed approaches, refuted assumptions, compatibility gaps, and known unsafe shortcuts.

### 11.2 Promotion ladder

```text
raw episode
→ curated lesson candidate
→ repeated support across independent episodes
→ contradiction and confound review
→ bounded applicability statement
→ shadow recommendation evaluation
→ admitted procedural/semantic/policy memory
→ monitored use with rollback
```

Memory never grants capability, satisfies a precondition, or overrides current observation. Every memory item cites source anchors and evidence digests and states the versions and fortress contexts to which it applies.

### 11.3 Handoff packet

A context-window or agent handoff contains:

- last acknowledged anchor;
- active mission/objectives;
- active work and required next protocol step;
- unresolved attention and uncertainty;
- capability/budget posture;
- decisions already rejected and why;
- compact evidence and memory references;
- no unverifiable narrative claims.

This makes agent replacement routine rather than catastrophic.

## 12. Multi-agent coherence

Multiple agents may inspect concurrently. Mutation requires both knowledge validity and authority fencing.

The agent packet therefore exposes:

- ownership leases and incarnations relevant to visible scopes;
- prepared-plan overlap and conflict summaries;
- objective ownership and delegation;
- shared observations versus private speculative branches;
- handoff and escalation records;
- deterministic merge/rebase recommendations.

Agents coordinate through semantic intents and evidence, not raw text locks. A lease does not make stale knowledge true; a valid witness does not confer ownership.

## 13. Error and recovery ergonomics

Every error is a valid Agent Turn Packet. In addition to stable code and message it states:

- whether the prior anchor remains usable;
- whether an effect may have occurred;
- active work that still exists;
- the exact retry/recovery class;
- the minimum safe next protocol step;
- information that would resolve the error;
- whether operator action is unavoidable.

Recovery classes are:

- `never_unchanged`;
- `safe_read_retry`;
- `refresh_and_retry`;
- `rebase_required`;
- `backoff`;
- `reconciliation_required`;
- `confirmation_required`;
- `operator_action_required`.

An error must not make the agent start over unless continuity is genuinely lost.

## 14. Budget accounting and progressive disclosure

Every request reports budget in a common ledger:

```text
requested ceiling
admitted ceiling
consumed by category
remaining
soft stop reason
hard stop reason
continuation availability
```

Categories include canonical reads, derived queries, bridge bytes, graph operations, search operations, planning expansions, candidate simulation, evidence materialization, output bytes, and output tokens.

When a budget is exhausted, the server returns the most decision-useful complete prefix plus coverage and continuation. It never truncates a structured object or hides a safety warning.

## 15. Determinism and semantic stability

The agent surface is deterministic under the pinned state, policy, seed, budget, and compatibility profile. This includes:

- field and item ordering;
- recommendation tie-breaks;
- attention tie-breaks;
- candidate ordering;
- omission priority under output budgets;
- continuation boundaries;
- explanation paths.

A deeper profile may add evidence or revise a lower-epistemic claim, but it cannot silently contradict an observed fact. Revisions are explicit events carrying old claim, new claim, reason, and evidence.

## 16. Non-goals and rejected shortcuts

The agent operating model explicitly rejects:

- one giant “state” response;
- free-form autonomous shell/DFHack/Lua recommendations;
- hidden server-side goals;
- opaque end-to-end “do everything” tools;
- unconstrained chain-of-thought storage;
- recommendations without alternatives and uncertainty;
- adaptive ranking without epochs and rollback;
- memory promotion from one anecdote;
- token savings achieved by omitting continuity, uncertainty, or safety state;
- prompting the agent to remember protocol facts the server can represent structurally;
- returning dozens of top-level tools instead of affordances inside a coherent schema.

## 17. Implementation sequence

### Gate A0: contract lock

- publish this document and the machine registry;
- add shared Agent Turn Packet builders;
- add golden shape and deterministic-order tests;
- add the packet to success and error responses without removing legacy fields.

### Gate A1: laboratory briefing

- `open_session` returns bootstrap orientation and supported profiles;
- `observe` supports `pulse` and `briefing` in the memory adapter;
- active plan/action state and safe next steps are always present;
- unchanged calls produce a heartbeat packet;
- errors preserve anchor and next-step guidance.

### Gate A2: epistemic and coverage semantics

- world facts carry epistemic status and provenance;
- absence requires complete-domain coverage;
- query/observe report omitted domains and continuations;
- stale/reset/gap behavior has golden vectors.

### Gate A3: affordances and objective slice

- registered pause/resume affordances are generated from state, grants, and risk;
- ephemeral objectives compile into candidate plans;
- plan output includes assumptions, predicted delta, costs, and invalidators;
- commit/wait compare prediction with observation and emit surprise.

### Gate A4: live read-only bridge orientation

- one live observation capsule produces the same packet as the laboratory;
- compatibility and bridge uncertainties are explicit;
- the packet is replayable from evidence;
- no live-only presentation path exists.

### Gate A5: verified live mutation and accretion

- pause/resume completes the full loop;
- episode and surprise records are durable;
- memory export is evidence-linked and authority-free;
- a fresh agent can resume from a handoff packet and complete the next safe step.

## 18. Acceptance tests from the agent's perspective

The system is not agent-intuitive merely because schemas validate. Acceptance scenarios include:

1. **Cold arrival:** an agent with no prior context opens a session and correctly identifies fortress state, limitations, authority, and safest next step from one response.
2. **Cheap heartbeat:** repeated `pulse` observations with no meaningful change stay compact and do not restate the world.
3. **Context loss:** a new agent receives a handoff and continues an obligation without replaying the full transcript.
4. **Cursor gap:** the agent is told exactly what remains valid and how to recover; no delta is silently bridged.
5. **Pending plan:** every response makes the pending plan and required commit/replan/cancel step visible.
6. **Indeterminate effect:** no recommendation permits blind retry; reconciliation dominates ranking.
7. **Unavailable action:** affordance output explains the missing capability or unmet precondition instead of merely omitting the action.
8. **Equivalent options:** candidate and recommendation ordering is byte-stable under the canonical tie-break policy.
9. **Budget pressure:** safety and continuity survive; optional detail moves behind a continuation.
10. **Learning:** an unexpected outcome creates a surprise record and lesson candidate but does not immediately alter production policy.
11. **Restore:** every stale handle is visibly invalidated and the new observation epoch is unmistakable.
12. **Live/lab parity:** the same semantic state yields the same agent packet independent of adapter implementation, except for declared provenance and compatibility fields.

The highest-level success criterion is simple: **the agent spends its cognition on fortress strategy, not on reconstructing the control plane.**
