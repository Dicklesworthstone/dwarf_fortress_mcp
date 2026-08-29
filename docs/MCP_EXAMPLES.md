# MCP Examples

These examples show intended semantics. Exact MCP framing may change during implementation; the
`dfmcp` objects are the design target.

## 1. Open a read-only session

Request:

```json
{
  "dfmcp_version": "0.1",
  "request_id": "request:00000000000000000000000000000001",
  "fortress": {"selector": "currently_loaded"},
  "requested_capabilities": [
    {
      "kind": "observe",
      "scope": {"fortress": "current"},
      "max_risk": "read_only"
    },
    {
      "kind": "query",
      "scope": {"fortress": "current"},
      "max_risk": "read_only"
    },
    {
      "kind": "plan",
      "scope": {"fortress": "current"},
      "max_risk": "read_only"
    }
  ],
  "observation_profile": "operations",
  "budget": {
    "wall_millis": 2000,
    "game_ticks": 10000,
    "max_entities": 2000,
    "max_bytes": 4194304,
    "max_output_tokens": 1500,
    "max_actions": 64
  }
}
```

Response sketch:

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000001",
  "anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20811},
    "game_tick": 9120031,
    "state_hash": "sha256:91c44f..."
  },
  "result": {
    "compatibility": {
      "level": "exact",
      "manifest": "compat:df-52.06-dfhack-52.06-r1-bridge-0.1"
    },
    "grants": ["observe", "query", "plan"],
    "disabled_action_families": [
      {"family": "military", "reason": "not certified in manifest"}
    ],
    "summary": {
      "population": 118,
      "paused": true,
      "critical_attention": [
        {
          "subject": "work_order:iron-bars",
          "score": 0.91,
          "reason": "fuel reserve below configured floor"
        }
      ]
    }
  },
  "evidence": ["evidence:7d..."],
  "warnings": [],
  "truncated": false,
  "continuation": null
}
```

## 2. Observe since prior cursor

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000002",
  "expected_anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20811},
    "state_hash": "sha256:91c44f..."
  },
  "since": {"epoch": 4, "sequence": 20811},
  "projection": "summary",
  "interest": {
    "entity_kinds": ["unit", "job", "work_order", "building"],
    "fields": ["state", "blocked_by", "stress", "health", "criticality"],
    "event_kinds": ["threat_detected", "job_changed", "announcement"],
    "operations": "owned"
  },
  "budget": {
    "wall_millis": 1000,
    "game_ticks": 0,
    "max_entities": 500,
    "max_bytes": 1048576,
    "max_output_tokens": 500,
    "max_actions": 1
  }
}
```

A complete delta response names both anchors and may say:

```json
{
  "mode": "delta",
  "base_anchor": {
    "cursor": {"epoch": 4, "sequence": 20811},
    "state_hash": "sha256:91c44f..."
  },
  "target_anchor": {
    "cursor": {"epoch": 4, "sequence": 20817},
    "game_tick": 9120480,
    "state_hash": "sha256:02a8bd..."
  },
  "changes": [
    {
      "op": "upsert_entity",
      "entity": {
        "id": "job:77411:g1",
        "revision": 8,
        "kind": "job",
        "fields": {
          "state": {"known": "suspended"},
          "blocked_by": {"known": ["material:coke"]}
        }
      }
    }
  ],
  "attention": [
    {
      "subject": "job:77411:g1",
      "score_micros": 910000,
      "ledger": [
        {"signal": "critical_work_order", "contribution_micros": 500000},
        {"signal": "blocked_age", "contribution_micros": 310000},
        {"signal": "fuel_floor", "contribution_micros": 100000}
      ]
    }
  ]
}
```

## 3. Query blocked critical jobs

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000003",
  "expected_anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20817},
    "state_hash": "sha256:02a8bd..."
  },
  "query": {
    "from": {"kind": "job"},
    "where": {
      "all": [
        {"field": "state", "op": "eq", "value": "suspended"},
        {"field": "criticality", "op": "gte", "value": 0.7}
      ]
    },
    "select": ["id", "job_type", "workshop", "blocked_by", "age_ticks"],
    "order_by": [
      {"field": "criticality", "direction": "desc"},
      {"field": "id", "direction": "asc"}
    ],
    "limit": 40
  },
  "max_output_tokens": 800
}
```

## 4. Plan an excavation

Planning does not mutate:

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000004",
  "expected_anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20817},
    "state_hash": "sha256:02a8bd..."
  },
  "intent": {
    "id": "intent:00000000000000000000000000000007",
    "summary": "Excavate a 5x7 dry stone room for a metalsmith's forge",
    "terminal_condition": {
      "all": [
        {
          "area_terrain_is": {
            "cuboid": {
              "min": {"x": 110, "y": 74, "z": 102},
              "max": {"x": 114, "y": 80, "z": 102}
            },
            "terrain": "open_floor"
          }
        }
      ]
    },
    "constraints": [
      {"max_risk": "guarded"},
      {
        "exclude_area": {
          "min": {"x": 100, "y": 60, "z": 99},
          "max": {"x": 130, "y": 90, "z": 101}
        }
      },
      {"require_checkpoint": true},
      {"deadline_game_tick": 9130000}
    ],
    "requested_actions": [
      {
        "kind": "designate_dig",
        "area": {
          "min": {"x": 110, "y": 74, "z": 102},
          "max": {"x": 114, "y": 80, "z": 102}
        },
        "mode": "mine",
        "postconditions": [
          {
            "designation_matches": {
              "area": {
                "min": {"x": 110, "y": 74, "z": 102},
                "max": {"x": 114, "y": 80, "z": 102}
              },
              "mode": "mine"
            }
          }
        ],
        "obligation": {
          "terminal": {
            "area_terrain_is": {
              "cuboid": {
                "min": {"x": 110, "y": 74, "z": 102},
                "max": {"x": 114, "y": 80, "z": 102}
              },
              "terrain": "open_floor"
            }
          },
          "failure": {
            "any": [
              {"area_has_magma": true},
              {"area_has_cave_in": true}
            ]
          },
          "deadline_game_tick": 9130000,
          "poll_interval_ticks": 120,
          "stable_for_observations": 2
        }
      }
    ]
  }
}
```

Plan response includes:

```json
{
  "plan_id": "plan:6eb15b...",
  "plan_digest": "sha256:6eb15b...",
  "source_anchor": {
    "cursor": {"epoch": 4, "sequence": 20817},
    "state_hash": "sha256:02a8bd..."
  },
  "expires_at_game_tick": 9121680,
  "max_risk": "guarded",
  "requires_checkpoint": true,
  "required_capabilities": ["designate", "checkpoint"],
  "leases": [
    {
      "domain": "map_write",
      "cuboid": {
        "min": {"x": 110, "y": 74, "z": 102},
        "max": {"x": 114, "y": 80, "z": 102}
      }
    }
  ],
  "steps": [
    {
      "step_id": 0,
      "action": "designate_dig",
      "idempotency_key": "sha256:b81f...",
      "postconditions": ["designation_matches"],
      "obligation": "required"
    }
  ],
  "predicted_diff": {
    "designation_tiles_added": 35,
    "terrain_change": "future_obligation"
  }
}
```

## 5. Commit exact plan

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000005",
  "expected_anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20817},
    "state_hash": "sha256:02a8bd..."
  },
  "plan_id": "plan:6eb15b...",
  "plan_digest": "sha256:6eb15b...",
  "idempotency_key": "sha256:commit-1f...",
  "confirmation_seal": "seal:operator-or-policy-bound-to-plan"
}
```

A successful response still distinguishes immediate and long-running state:

```json
{
  "plan_id": "plan:6eb15b...",
  "checkpoint": {
    "checkpoint_id": "checkpoint:513dbc...",
    "seal": "sha256:513dbc...",
    "durable": true
  },
  "actions": [
    {
      "action_id": "action:6eb15b:0",
      "state": "applied_awaiting_verification",
      "immediate_postconditions": "verified",
      "obligation_id": "obligation:6eb15b:0:excavation",
      "evidence": ["evidence:designation-readback"]
    }
  ]
}
```

## 6. Wait for obligation

```json
{
  "dfmcp_version": "0.1",
  "session_id": "session:01J6KJ7JD0A4T7R61A4P3H0X9F",
  "request_id": "request:00000000000000000000000000000006",
  "expected_anchor": {
    "fortress_id": "fortress:world-44:site-17",
    "cursor": {"epoch": 4, "sequence": 20819},
    "state_hash": "sha256:..."
  },
  "targets": ["obligation:6eb15b:0:excavation"],
  "stop_when": ["terminal", "blocked_changed", "high_severity_event"],
  "budget": {
    "wall_millis": 2000,
    "game_ticks": 1200,
    "max_entities": 500,
    "max_bytes": 1048576,
    "max_output_tokens": 500,
    "max_actions": 1
  }
}
```

Possible blocked response:

```json
{
  "obligation": {
    "id": "obligation:6eb15b:0:excavation",
    "state": "blocked",
    "progress": {"completed_tiles": 28, "total_tiles": 35},
    "blockers": [
      {
        "kind": "dangerous_creature",
        "subject": "creature:giant-cave-spider:991:g1",
        "area": "remaining-work-area"
      }
    ]
  },
  "suggested_next_protocol_step": "plan a separate blocker-resolution intent"
}
```

## 7. Cursor reset after restore

A client that supplies epoch 4 after restore receives:

```json
{
  "error": {
    "code": "ERR-CURSOR-GAP",
    "retry_class": "refresh_and_retry",
    "message": "observation epoch changed after checkpoint restore",
    "current_anchor": {
      "cursor": {"epoch": 5, "sequence": 0},
      "state_hash": "sha256:..."
    },
    "required_action": "request a full snapshot"
  }
}
```

## 8. Indeterminate mutation

```json
{
  "action_id": "action:6eb15b:0",
  "state": "indeterminate",
  "reason": "bridge connection lost after durable dispatch intent and before receipt",
  "automatic_retry_allowed": false,
  "reconciliation": {
    "bridge_journal": "unavailable_after_bridge_restart",
    "semantic_markers": ["designation tile mask"],
    "next_check": "refresh exact map chunks and event window"
  },
  "evidence": [
    "evidence:dispatch-intent",
    "evidence:transport-loss",
    "evidence:bridge-instance-change"
  ]
}
```

The correct next action is reconciliation, not a duplicate `commit`.
