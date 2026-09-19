#!/usr/bin/env python3
"""Source wiring and independent pagination/JSON-size models, NOT Rust execution."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
BASE = 16 * 1024
RECORD = 64 * 1024


def json_bytes(value: object) -> int:
    return len(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode())


def lexical_delimiters(text: str) -> None:
    """Only comments, string/char extents and delimiters; not a Rust parser."""
    i, stack = 0, []
    pairs = {")": "(", "]": "[", "}": "{"}
    while i < len(text):
        if text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end + 1
        elif text.startswith("/*", i):
            depth, i = 1, i + 2
            while depth and i < len(text):
                if text.startswith("/*", i): depth, i = depth + 1, i + 2
                elif text.startswith("*/", i): depth, i = depth - 1, i + 2
                else: i += 1
            assert depth == 0, "unclosed block comment"
        elif text[i] == '"':
            i += 1
            while i < len(text):
                if text[i] == "\\": i += 2
                elif text[i] == '"': i += 1; break
                else: i += 1
            else: raise AssertionError("unclosed string")
        elif text[i] == "'" and (match := re.match(r"'(?:[^'\\\n]|\\(?:[nrt0\\'\"]|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}))'", text[i:])):
            i += len(match[0])
        elif text[i] in "([{": stack.append(text[i]); i += 1
        elif text[i] in ")]}":
            assert stack and stack.pop() == pairs[text[i]], f"delimiter mismatch at {i}"
            i += 1
        else: i += 1
    assert not stack, "unclosed delimiters"


def pagination_cases() -> int:
    cases = 0
    for count in [0, 1, 7, 8, 16, 63, 64, 65, 129, 257, 4096]:
        for period in [1, 3, 67]:
            states = ["prepared" if i % period == 0 else "created" for i in range(count)]
            for page_limit in [1, 2, 8]:
                for scan_limit in [1, 7, 64]:
                    after, collected, calls = -1, [], 0
                    while True:
                        page = list(range(after + 1, min(count, after + 1 + scan_limit)))
                        emitted, consumed = [], 0
                        for index in page:
                            consumed += 1
                            if states[index] == "prepared": emitted.append(index)
                            if len(emitted) == page_limit: break
                        collected += emitted
                        more = consumed < len(page) or (page and page[-1] + 1 < count)
                        calls += 1
                        assert len(emitted) <= page_limit and calls <= count + 1
                        if not more: break
                        next_after = page[consumed - 1]
                        assert next_after > after  # Including an empty filtered page.
                        after = next_after
                    assert collected == [i for i in range(count) if states[i] == "prepared"]
                    assert len(collected) == len(set(collected))
                    cases += 1
    return cases


def sizing_reference() -> tuple[int, int, int]:
    # This deliberately combines the longest allowed scalar/string forms and
    # receipt fields even where that combination is not a legal native state.
    # It is an upper-bound JSON model, not a captured MCP response.
    digest = "f" * 64
    observation = {
        "kind": "native_order_queue_membership", "fortress_id": str(2**64 - 1),
        "bridge_generation": 2**64 - 2, "intervention_sequence": 2**64 - 3,
        "game_tick": (2**32 - 1) * 403200 + 403199, "world_folder": "\x01" * 512,
        "site_id": 2**31 - 1, "paused": True, "next_order_id": 2**31 - 2,
        "order_ids": list(range(2**31 - 5000, 2**31 - 905)), "witness": digest,
        "eligible_at_observation": False, "complete_queue_membership": True,
        "existing_order_configurations_observed": False, "production_feasibility_proven": False,
    }
    record = {
        "idempotency_key": "x" * 128, "plan_digest": digest, "recipe": "wooden_chair", "amount": 100,
        "state": "cancelled_before_dispatch", "original_observation": observation,
        "native_state": "prepared", "created_order_id": 2**31 - 2,
        "observed_tick": observation["game_tick"], "after_witness": digest,
        "configuration_witness": digest, "receipt": digest, "native_effect_hex": "f" * (357 * 2),
        "source": {"df_version": "\x01" * 128, "dfhack_version": "\x02" * 128,
                   "generation": 2**64 - 2, "protocol": "1.10"},
        "reconciliation_required": False, "safe_to_retry_insertion": False,
        "production_goal_completion_proven": False, "current_freshness_proven": False,
    }
    record_size = json_bytes(record)
    assert record_size < RECORD
    # Model the full projected envelope keys and explicit recovery metadata.
    summary = {"journal_id": digest, "head": digest, "fortress_id": str(2**64 - 1),
               "records": 4096, "prepared": 4096, "unresolved": 4096, "terminal": 4096,
               "transitions": 16384, "retained_bytes": 64*1024*1024,
               "mode": "reconcile", "restart_recovery": True,
               "scope": "creation_coordination_not_game_history"}
    discovery = {"tool": "fortress.query", "arguments": {"session_id": "f"*32, "state": "pending", "limit": 2}}
    active = {"pending_plans": [{"retained_count":4096,"discover":discovery}], "actions": [],
              "obligations": [], "cancellation_drains": [], "indeterminate_effects": [{"retained_count":4096,"discover":discovery}],
              "publications": [], "confirmations": [], "scope": "this_creation_journal_only",
              "state_known": False, "prepared_count": 4096, "unresolved_count": 4096,
              "terminal_count": 4096, "details_omitted": 8192, "discovery": discovery}
    turn = {"schema":"dfmcp.agent_turn/1", "operation":"fortress.query", "phase":"reconcile",
            "session_id":"f"*32, "request_id":"f"*32, "turn_id":None,
            "anchor":{"kind":"selected_native_queue_not_canonical_world","fortress_id":str(2**64-1),
                      "bridge_generation":2**64-2,"intervention_sequence":2**64-3,
                      "game_tick":observation["game_tick"],"witness":digest},
            "continuity":{"status":"indeterminate","basis":None,"gap":None,"reset_reason":None},
            "profile":"briefing", "briefing":{"runtime":"unadmitted_development","bridge_protocol":"1.10",
                "mode":"reconcile","runtime_admitted":False,"mutation_admissible":False,
                "development_production_granted":False,"current_freshness_proven":False,
                "native_created_is_not_production_completion":True},
            "changes":[],"attention":[],"active_work":active,
            "affordances":[{"action":"work_order.create","enabled":False,
                "recipes":["wooden_bed","wooden_door","wooden_table","wooden_chair"],"amount_min":1,"amount_max":100,
                "requires":"current exact queue observation, sealed plan and independent commit authorization"}],
            "recommendations":[dict(discovery,reason="discover retained work before new creation; unresolved attempts cannot be retried")],
            "coverage":{"status":"partial","complete_domains":[],"partial_domains":["retained_creation_coordination","selected_order_queue_membership"],
                "omitted_domains":["production_progress","order_approval","material_availability","world_state","other_controllers"]},
            "uncertainty":[{"code":"development_evidence_only","detail":"No admitted runtime, current world snapshot, completed production or other-controller safety is established."},
                {"code":"retained_work_scope","detail":"Absent summary means custody or authority is unverified, not that no work exists."}],
            "budget":{"admitted":{"max_bytes":68*1024*1024,"max_output_tokens":262144,"max_wall_millis":60000},
                "token_accounting":"four_byte_proxy_not_tokenizer_count"},"references":[]}
    result = {"ok":False,"records":[],"scanned_records":64,"state":"reconciliation_required",
              "matching_records_in_journal":4096,"continuation":digest,
              "complete_matching_set_in_this_response":False,"journal":summary}
    base = json_bytes({"agent_turn":turn,"result":result})
    assert base < BASE
    for rows in range(1,9):
        result["records"] = [record] * rows
        assert json_bytes({"agent_turn":turn,"result":result}) < BASE + RECORD * rows
    return record_size, base, json_bytes({"agent_turn":turn,"result":result})


def main() -> None:
    paths = [ROOT/'crates/dfmcp-mcp/src'/name for name in [
        'live_work_orders_server.rs','work_order_presentation.rs','live_work_orders_server_tests.rs',
        'work_order_presentation_tests.rs','bin/dfmcp-live-work-orders-dev-server.rs']]
    paths += [ROOT/'crates/dfmcp-mcp/src/lib.rs',Path(__file__)]
    for path in paths[:-1]:
        text = path.read_text(encoding='utf-8')
        assert '\0' not in text
        lexical_delimiters(text)
    source = paths[0].read_text()
    tools = re.findall(r'\.tool\((Fortress\w+)\)', source)
    assert tools == ['FortressOpenSession','FortressObserve','FortressQuery','FortressPlan','FortressCommit',
                     'FortressWait','FortressCancel','FortressCheckpoint','FortressRestore','FortressExplain','FortressDoctor']
    assert source.count('#[tool(') == 11
    assert 'pub mod live_work_orders_server;' in paths[-2].read_text()
    assert 'dfmcp_mcp::live_work_orders_server::run_stdio();' in paths[4].read_text()
    assert source.index('if mode == JournalMode::Offline { None } else') < source.index('configured("DFMCP_WORK_ORDERS_TOKEN"')
    assert 'self.control.commit(&key, plan, witness, context)' in source
    assert 'commit_prepared(' not in source
    assert '"DFMCP_WORK_ORDERS_ALLOW_PRODUCTION"' in source
    counts = sum(path.read_text().count('#[test]') for path in paths[:5])
    assert counts == 16
    cases = pagination_cases()
    record_size, base_size, largest_page = sizing_reference()
    print(json.dumps({'schema':'dfmcp.work-order-mcp-reference/1','status':'passed_reference_only',
        'pagination_model_cases':cases,'maximal_record_model_bytes':record_size,
        'envelope_model_bytes':base_size,'eight_record_page_model_bytes':largest_page,
        'record_reservation_bytes':RECORD,'envelope_reservation_bytes':BASE,
        'mcp_tools_wired':len(tools),'rust_tests_registered':counts,
        'rust_compiled':False,'rust_tests_executed':False,'mcp_executed':False,
        'filesystem_or_native_game_executed':False,
        'scope':'Independent Python pagination/JSON-size models and lexical/source wiring only; not a Rust parser or runtime test',
        'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}},
        indent=2,sort_keys=True))


if __name__ == '__main__': main()
