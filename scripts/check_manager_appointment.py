#!/usr/bin/env python3
"""Manager/1.22 source/codec inventory gate, not SDK, Rust or live proof.

Consumer: Manager component builds and the repository static-contract gates.
Defect class: missing/stale generation inputs or an unregistered semantic route.
Delete with removal of this development feature; this grants no admission.
"""
from pathlib import Path
import json
import re
from read_stable_repository_file import read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
A = "crates/dfmcp-adapter/src/"
M = "crates/dfmcp-mcp/"
METHODS = ["Handshake", "ReadManager", "PrepareManager", "CommitManager", "QueryManager", "CancelManager"]
TOOLS = ["fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "checkpoint", "restore", "explain", "doctor")]
AUTHORITY_TESTS = [
    "manager_prepare_requires_authority_at_capture_tick_without_prior_observe",
    "manager_plan_recovery_edges_preserve_observed_tick_high_water",
    "manager_dispatch_guard_and_writer_use_retained_tick_for_actual_lease",
]
SOURCES = [
    "architecture/manager_appointment_v1_22.json",
    "bridge/common/manager_appointment.h", "bridge/common/one_attempt_native.h",
    "bridge/common/workshop_construction.h", "bridge/common/retained_snapshot.h",
    *["bridge/dfhack-manager-v1_22/" + n for n in ("CMakeLists.txt", "DfmcpManagerV1_22.proto", "dfmcp_manager_v1_22.cpp")],
    A + "lib.rs", A + "bounded_run/mod.rs", A + "order_run/mod.rs",
    *[A + "manager_appointment/" + n for n in ("mod.rs", "capture.rs", "record.rs", "rpc.rs", "journal.rs", "tests.rs", "rpc_tests.rs", "journal_tests.rs")],
    A + "build_placement/rpc.rs",
    *[A + "build_placement/rpc/" + n for n in ("codec.rs", "link.rs")],
    A + "build_placement/journal/private_file.rs", A + "workshop_construction/acknowledgement.rs", A + "workshop_construction/journal.rs",
    M + "src/lib.rs", M + "src/agent_turn.rs", M + "src/manager_appointment_server.rs",
    *[M + "src/manager_appointment_server/" + n for n in ("runtime.rs", "presentation.rs", "server_tests.rs")],
    M + "src/bin/dfmcp-manager-v1-22-dev-server.rs", M + "tests/manager_appointment_stdio.rs",
    "crates/dwarf-fortress-mcp/src/main.rs", "crates/dwarf-fortress-mcp/tests/manager_gameplay_cli.rs",
    "tests/native/manager/stubs.h", "tests/native/manager/handler.cpp", "tests/fixtures/manager_v1_22_component.json",
    "tests/stdio/manager_appointment.py", "scripts/read_stable_repository_file.py",
    "scripts/check_manager_appointment.py", "scripts/test_manager_source_inventory.py", "scripts/test_manager_native.py",
    "docs/MANAGER_APPOINTMENT_MCP.md", "scripts/verify.sh", "scripts/qualify_local.sh",
]


def check(root=ROOT):
    texts, hashes = {}, {}
    for name in SOURCES:
        value = read_stable_regular_file(root / name, 4 * 1024 * 1024, name)
        texts[name] = value.content.decode("utf-8", errors="strict")
        if "\0" in texts[name]:
            raise ValueError("NUL in Manager source: " + name)
        hashes[name] = value.sha256
    contract = json.loads(texts[SOURCES[0]])
    if contract["source_paths"] != SOURCES or contract["native_methods"] != METHODS:
        raise ValueError("Manager exact generation inventory differs")
    if contract["production_admitted"] is not False or contract["status"] != "implemented_unadmitted_development_only":
        raise ValueError("Manager development source cannot grant admission")
    if contract["tool_names"] != TOOLS or contract["protocol"] != [1, 22]:
        raise ValueError("Manager semantic waist/generation differs")
    native = texts["bridge/dfhack-manager-v1_22/dfmcp_manager_v1_22.cpp"]
    if re.findall(r's->addFunction\("([^"]+)"', native) != METHODS or '#include "df/gamest.h"' not in native:
        raise ValueError("Manager native manifest or actual SDK type differs")
    proto = texts["bridge/dfhack-manager-v1_22/DfmcpManagerV1_22.proto"]
    if "package dfmcp.manager.v1_22;" not in proto:
        raise ValueError("Manager wire package differs")
    rpc = texts[A + "manager_appointment/rpc.rs"]
    start = rpc.index("pub const METHODS:")
    if re.findall(r'"([A-Za-z]+)"', rpc[start:rpc.index("];", start)]) != METHODS:
        raise ValueError("Manager Rust method list differs")
    if contract.get("focused_authority_tests") != AUTHORITY_TESTS:
        raise ValueError("Manager paired authority regression inventory differs")
    authority_tests = texts[A + "manager_appointment/rpc_tests.rs"]
    for name in AUTHORITY_TESTS:
        if re.search(r"#\[test\]\s*fn\s+" + re.escape(name) + r"\s*\(", authority_tests) is None:
            raise ValueError("Manager focused authority regression unregistered: " + name)
    server = texts[M + "src/manager_appointment_server.rs"]
    if sorted(re.findall(r'pub fn (fortress_\w+)\(', server)) != sorted(TOOLS):
        raise ValueError("Manager must register exactly eleven handlers")
    if '"manager" => dfmcp_mcp::manager_appointment_server::run_stdio()' not in texts["crates/dwarf-fortress-mcp/src/main.rs"]:
        raise ValueError("Manager standard CLI route missing")
    packaging = contract.get("mcp_result_packaging")
    if packaging != {"raw_chunk_bytes": 24000, "complete_jsonrpc_bytes_max": 524288,
            "jsonrpc_id_bytes_reserved": 4096, "encoded_string_id_bytes_max": 256,
            "numeric_id_lexeme_bytes_max": 4096, "transport_limits_changed": False}:
        raise ValueError("Manager bounded result packaging differs")
    presentation = re.sub(r"\s+", "", texts[M + "src/manager_appointment_server/presentation.rs"])
    if "constMAX_JSONRPC_ID_BYTES:usize=4096;" not in presentation or "+MAX_JSONRPC_ID_BYTES+" not in presentation:
        raise ValueError("Manager must reserve admitted numeric IDs within unchanged product limit")
    vectors = "tests/fixtures/manager_v1_22_component.json"
    if hashes[vectors] != contract["component_vector_file_sha256"]:
        raise ValueError("Manager corrected native component vector bytes differ")
    for name, declaration in ((A + "lib.rs", "pub mod manager_appointment;"), (M + "src/lib.rs", "pub mod manager_appointment_server;")):
        if declaration not in texts[name]:
            raise ValueError("Manager module unregistered")
    for name in ("scripts/verify.sh", "scripts/qualify_local.sh"):
        if "python3 scripts/test_manager_native.py --mutations" not in texts[name]:
            raise ValueError("Manager native regression gate missing: " + name)
    expected_review_budget = {
        "bead": "dfmcp-reality-wiring-proof-4bf.14.3",
        "default_wall_millis": 60000, "maximum_wall_millis": 60000,
        "connection_deadline_renewal": False,
        "pre_dispatch_expiry": "reject before durable dispatch; consume local permission; retain original preparation custody for reconciliation",
        "after_dispatch_failure": "indeterminate original effect; no automatic retry",
    }
    if contract.get("prepared_review_budget") != expected_review_budget:
        raise ValueError("prepared review budget contract differs")
    if "unwrap_or(60000)" not in texts["crates/dfmcp-mcp/src/manager_appointment_server.rs"]:
        raise ValueError("prepared review default differs")
    runtime = texts["crates/dfmcp-mcp/src/manager_appointment_server/runtime.rs"]
    if runtime.index("source.preflight_commit(") >= runtime.index("let record = self.journal.dispatch("):
        raise ValueError("prepared connection preflight must precede durable dispatch")
    return {"schema": "dfmcp.manager-source-check/1", "status": "passed_static_only", "source_sha256": hashes,
            "source_files": len(SOURCES), "adapter_modules": 8, "rust_executed": False, "sdk_or_game_executed": False}


if __name__ == "__main__":
    print(json.dumps(check(), sort_keys=True))
