#!/usr/bin/env python3
"""Mandatory static Tree/1.21 consumer/input gate, never execution or admission.

Both qualification wrappers consume this check. It detects missing consumer,
custody-before-writer, test registration and source inventory. Delete it only
with removal of this isolated development feature.
"""
from pathlib import Path
import ast
import hashlib
import json
import re
from read_stable_repository_file import read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
A = "crates/dfmcp-adapter/src/"
M = "crates/dfmcp-mcp/"
METHODS = ["Handshake", "ReadTreeHarvest", "PrepareTreeHarvest", "CommitTreeHarvest", "QueryTreeHarvest", "CancelTreeHarvest", "ReadTreeHarvestProgress"]
TOOLS = ["fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "checkpoint", "restore", "explain", "doctor")]
SOURCES = [
    "architecture/tree_harvest_v1_21.json", "bridge/common/tree_harvest.h", "bridge/common/one_attempt_native.h",
    "bridge/common/workshop_construction.h", "bridge/common/retained_snapshot.h",
    *["bridge/dfhack-tree-harvest-v1_21/" + n for n in ("CMakeLists.txt", "DfmcpTreeHarvestV1_21.proto", "dfmcp_tree_harvest_v1_21.cpp")],
    A + "lib.rs", A + "bounded_run/mod.rs", A + "order_run/mod.rs",
    *[A + "build_placement/rpc/" + n for n in ("codec.rs", "link.rs")],
    A + "build_placement/journal/private_file.rs", A + "workshop_construction/acknowledgement.rs",
    *[A + "tree_harvest/" + n for n in ("mod.rs", "rpc.rs", "journal.rs", "material_origin.rs", "tests.rs", "rpc_tests.rs", "journal_tests.rs")],
    M + "src/lib.rs", M + "src/tree_harvest_server.rs", M + "src/tree_harvest_server/runtime.rs",
    M + "src/tree_harvest_server/presentation.rs",
    M + "src/tree_harvest_server/server_tests.rs", M + "src/bin/dfmcp-tree-harvest-v1-21-dev-server.rs",
    M + "tests/tree_harvest_stdio.rs", "tests/native/workshop/stubs.h",
    *["tests/native/tree_harvest/" + n for n in ("stubs.h", "handler.cpp", "vectors.json")],
    "tests/stdio/tree_harvest.py", "scripts/read_stable_repository_file.py", "scripts/test_tree_harvest_native.py",
    "scripts/check_tree_harvest.py", "scripts/test_tree_harvest_source_inventory.py",
    "scripts/verify.sh", "scripts/qualify_local.sh", "docs/TREE_HARVEST_MCP.md",
]


def check(root: Path = ROOT) -> dict:
    texts, hashes = {}, {}
    for name in SOURCES:
        value = read_stable_regular_file(root / name, 4 * 1024 * 1024, name)
        texts[name] = value.content.decode("utf-8", errors="strict")
        if "\0" in texts[name]:
            raise ValueError(f"NUL in tree source: {name}")
        hashes[name] = value.sha256
    contract = json.loads(texts[SOURCES[0]])
    if contract["source_paths"] != SOURCES or contract["native_methods"] != METHODS:
        raise ValueError("tree exact source/method inventory differs")
    if contract["status"] != "implemented_unadmitted_development_only" or contract["production_admitted"] is not False:
        raise ValueError("tree development status cannot grant admission")
    if contract["mcp_consumer"]["tool_names"] != TOOLS or contract["plant_stable_id"] is not None:
        raise ValueError("tree waist or unknown identity differs")
    compact = {p: re.sub(r"\s+", "", t) for p, t in texts.items()}
    required = {
        A + "lib.rs": ["pubmodtree_harvest;"],
        M + "src/lib.rs": ["pubmodtree_harvest_server;"],
        A + "tree_harvest/mod.rs": ["pubmodjournal;", "pubmodrpc;", "pub(crate)modtests;"],
        A + "tree_harvest/journal.rs": ["pub(crate)modjournal_tests;", "ack.acknowledge(self.pin(),&w.reserve(crate::workshop_construction::acknowledgement::WORK_BYTESasusize)?,)?;", "letpermitted=self.dispatchable();self.permit=false;"],
        A + "tree_harvest/material_origin.rs": ["self.mode==Mode::Offline", "work.charge(raw.len())?;", "acknowledgement.verify_tree(", "check(raw==self.raw)?;", "sample.generation==self.binding.generation", "last_state==State::Terminalasu8&&!pending_sample", "require(record.phase==2,"],
        A + "tree_harvest/rpc.rs": ["modrpc_tests;", "self.prepared.take().as_ref()==Some(p)", "progress.generation==self.binding.generation", "consumed_tree_tick"],
        M + "src/tree_harvest_server.rs": ["modserver_tests;", "modpresentation;", "spine(operation,session,request)", "\"fortress_plan\"", "\"fortress_commit\"", "reference[\"uncertain_effect\"]==true"],
        M + "src/tree_harvest_server/presentation.rs": ["constCHUNK_BYTES:usize=24_000;", "ResultMeta::empty()", "CoreResult::Final(FinalCoreResult::ToolsCall{", ".encode()", "ENVELOPE_BYTES", ".len()+4096+", "\"dfmcp.tree-native-capture/1\"", "Digest32::of_bytes(chunk)", "overflow_packet("],
        M + "src/tree_harvest_server/runtime.rs": ["self.withdraw();", "acks.latest()", "work.reserve(&current,32*1024*1024)", "journal::open_private(", "c.authorize(Capability::Query,RiskTier::ReadOnly,&[],Some(area))?;", "nonce(c,self.scope)?", "nonce(&work.reserve(&current,32)?,config.scope)?", "c.budget.max_bytes<32"],
        M + "src/bin/dfmcp-tree-harvest-v1-21-dev-server.rs": ["dfmcp_mcp::tree_harvest_server::run_stdio();", "std::process::ExitCode::FAILURE"],
    }
    for name, tokens in required.items():
        if any(token not in compact[name] for token in tokens):
            raise ValueError(f"tree required boundary missing: {name}")
    server = texts[M + "src/tree_harvest_server.rs"]
    handlers = re.findall(r"pub fn (fortress_\w+)\(", server)
    if len(handlers) != 11 or set(handlers) != set(TOOLS):
        raise ValueError("tree eleven handlers differ")
    if server.count("-> McpResult<CompleteResult<FinalCallToolResult>>") != 11:
        raise ValueError("tree tools must return explicit typed modern results")
    packaging = {
        "schema": "dfmcp.tree-native-capture/1", "content_zero": "complete_agent_turn_ack_and_descriptors",
        "encoding": "hex", "raw_chunk_bytes": 24000, "result_string_bytes_max": 65536,
        "complete_jsonrpc_bytes_max": 524288, "jsonrpc_encoded_string_id_bytes_max": 256, "jsonrpc_numeric_id_bytes_max": 4096,
        "binding": ["field_path", "byte_length", "sha256", "ordered_content_index", "offset_bytes", "chunk_byte_length", "chunk_sha256"],
        "consumer_must_verify_all_blocks": True, "transport_limits_changed": False,
    }
    if contract.get("mcp_result_packaging") != packaging:
        raise ValueError("tree result packaging contract differs")
    for name in ("OpenSession", "Observe", "Query", "Plan", "Commit", "Wait", "Cancel", "Checkpoint", "Restore", "Explain", "Doctor"):
        if f".tool(Fortress{name})" not in compact[M + "src/tree_harvest_server.rs"]:
            raise ValueError("unregistered tree tool")
    journal = compact[A + "tree_harvest/journal.rs"]
    dispatch = journal.index("self.append_work(State::Dispatched,")
    writer = journal.index("letrecord=writer(", dispatch)
    if not dispatch < journal.index("guard(&work.current()?)?;", dispatch) < writer:
        raise ValueError("tree pre-dispatch ACK/guard ordering differs")
    native = ast.parse(texts["scripts/test_tree_harvest_native.py"])
    constants = {}
    for node in native.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            try:
                constants[node.targets[0].id] = ast.literal_eval(node.value)
            except (ValueError, TypeError):
                pass
            if node.targets[0].id == "INPUTS":
                constants["INPUTS"] = [constants[v.id] if isinstance(v, ast.Name) else ast.literal_eval(v) for v in node.value.elts]
            if node.targets[0].id == "MUTATIONS":
                constants["MUTATION_NAMES"] = [ast.literal_eval(k) for k in node.value.keys]
    if constants.get("INPUTS") != contract["native_component_source_paths"] or constants.get("MUTATION_NAMES") != contract["native_mutants"]:
        raise ValueError("tree native source/mutant inventory differs")
    cpp = texts["tests/native/tree_harvest/handler.cpp"]
    if re.findall(r'run\("([^"]+)",', cpp) != contract["native_scenarios"]:
        raise ValueError("tree native scenarios differ")
    if hashlib.sha256(texts["tests/native/tree_harvest/vectors.json"].encode()).hexdigest() != contract["native_vectors_sha256"]:
        raise ValueError("original tree vectors changed")
    rust = [A + "tree_harvest/" + n for n in ("tests.rs", "journal_tests.rs", "rpc_tests.rs")]
    unit = [M + "src/tree_harvest_server/server_tests.rs"]
    integration = [M + "tests/tree_harvest_stdio.rs"]
    actual = {
        "adapter_rust_cases": [name for p in rust for name in re.findall(r"#\[test\]\s*fn (\w+)", texts[p])],
        "mcp_unit_cases": [name for p in unit for name in re.findall(r"#\[test\]\s*fn (\w+)", texts[p])],
        "rust_stdio_cases": [name for p in integration for name in re.findall(r"#\[test\]\s*fn (\w+)", texts[p])],
        "python_stdio_cases": re.findall(r"^    def (test_\w+)\(", texts["tests/stdio/tree_harvest.py"], re.MULTILINE),
        "python_inventory_cases": re.findall(r"^    def (test_\w+)\(", texts["scripts/test_tree_harvest_source_inventory.py"], re.MULTILINE),
    }
    if actual != contract["focused_test_cases"]:
        raise ValueError(f"registered tree test names differ: {actual}")
    for wrapper in ("scripts/verify.sh", "scripts/qualify_local.sh"):
        if any(name not in texts[wrapper] for name in ("check_tree_harvest.py", "test_tree_harvest_source_inventory.py", "test_tree_harvest_native.py --mutations", "tests/stdio/tree_harvest.py")):
            raise ValueError("tree mandatory gate wiring absent")
    expected_material_import = {
        "consumer": "workshop/1.20 material discovery",
        "owner_mode": "offline",
        "requires_current_independent_tree_ack": True,
        "original_designation_receipt_required": True,
        "pending_or_indeterminate_history_accepted": False,
        "retained_source_floors": "derive from exact privately retained original and every consumed progress frame",
        "canonical_history_rewritten": False,
        "native_calls_or_custody_writes": False,
        "restores_preparation_or_dispatch_permission": False,
        "current_material_or_causal_provenance_proved": False,
    }
    if contract.get("read_only_material_import") != expected_material_import:
        raise ValueError("Tree read-only material import contract differs")
    expected_review_budget = {
        "bead": "dfmcp-reality-wiring-proof-4bf.14.3",
        "default_wall_millis": 60000, "maximum_wall_millis": 60000,
        "connection_deadline_renewal": False,
        "pre_dispatch_expiry": "reject before durable dispatch; consume local permission; retain original preparation custody for reconciliation",
        "after_dispatch_failure": "indeterminate original effect; no automatic retry",
    }
    if contract.get("prepared_review_budget") != expected_review_budget:
        raise ValueError("prepared review budget contract differs")
    if "unwrap_or(60000)" not in texts["crates/dfmcp-mcp/src/tree_harvest_server.rs"]:
        raise ValueError("prepared review default differs")
    runtime = texts["crates/dfmcp-mcp/src/tree_harvest_server/runtime.rs"]
    if runtime.index("source.preflight_commit(") >= runtime.index("let record = self.journal.dispatch("):
        raise ValueError("prepared connection preflight must precede durable dispatch")
    return {"status": "passed_static_only", "source_sha256": hashes, "rust_executed": False, "sdk_or_game_executed": False, "production_admitted": False}


if __name__ == "__main__":
    print(json.dumps(check(), sort_keys=True, indent=2))
