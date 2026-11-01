#!/usr/bin/env python3
"""Static workshop/1.20 wiring gate; not Rust, SDK, game or admission proof.

Consumed by both mandatory verification commands. Rejects missing or drifted
registered consumer, fixed input inventory, acknowledgement-before-writer and
modern tool contracts. Remove only when this development profile is removed.
"""
from pathlib import Path
import ast
import json
import re
from read_stable_repository_file import read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = "crates/dfmcp-adapter/src/workshop_construction/"
MCP = "crates/dfmcp-mcp/src/"
METHODS = ["Handshake", "ReadWorkshop", "PrepareWorkshop", "CommitWorkshop", "QueryWorkshop", "CancelWorkshop", "ReadWorkshopProgress"]
TOOLS = ["fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "checkpoint", "restore", "explain", "doctor")]
SOURCES = [
    "architecture/workshop_construction_v1_20.json", "bridge/common/retained_snapshot.h", "bridge/common/workshop_construction.h",
    "bridge/dfhack-workshop-v1_20/CMakeLists.txt", "bridge/dfhack-workshop-v1_20/DfmcpWorkshopV1_20.proto",
    "bridge/dfhack-workshop-v1_20/dfmcp_workshop_v1_20.cpp", "crates/dfmcp-adapter/src/lib.rs",
    "crates/dfmcp-adapter/src/build_placement/rpc/codec.rs", "crates/dfmcp-adapter/src/build_placement/rpc/link.rs",
    *[ADAPTER + name for name in ("mod.rs", "acknowledgement.rs", "journal.rs", "journal_tests.rs", "rpc.rs", "rpc_tests.rs", "tests.rs")],
    MCP + "lib.rs", MCP + "workshop_construction_server.rs",
    *[MCP + "workshop_construction_server/" + name for name in ("runtime.rs", "presentation.rs", "tests.rs")],
    MCP + "bin/dfmcp-workshop-v1-20-dev-server.rs", "crates/dfmcp-mcp/tests/workshop_construction_stdio.rs",
    *["tests/native/workshop/" + name for name in ("stubs.h", "handler.cpp", "vectors.json", "continuity_vectors.json")],
    "tests/stdio/workshop.py", "scripts/read_stable_repository_file.py", "scripts/test_workshop_native.py",
    "scripts/test_workshop_source_inventory.py", "scripts/check_workshop_construction.py", "scripts/verify.sh",
    "scripts/qualify_local.sh", "docs/WORKSHOP_CONSTRUCTION_MCP.md",
]


def check(root: Path = ROOT) -> dict:
    text = {}
    hashes = {}
    for name in SOURCES:
        value = read_stable_regular_file(root / name, 4 * 1024 * 1024, name)
        text[name] = value.content.decode("utf-8", errors="strict")
        if "\0" in text[name]:
            raise ValueError(f"NUL in workshop source: {name}")
        hashes[name] = value.sha256
    contract = json.loads(text[SOURCES[0]])
    if contract["source_paths"] != SOURCES or contract["methods"] != METHODS:
        raise ValueError("workshop exact source inventory or method list differs")
    if contract["status"] != "implemented_unadmitted_development_only" or contract["production_admitted"] is not False:
        raise ValueError("workshop development source cannot grant admission")
    if contract["mcp_consumer"]["tool_names"] != TOOLS or contract["mcp_consumer"]["canonical_world_adapter"] is not False:
        raise ValueError("workshop tool or canonical continuity contract differs")
    compact = {name: re.sub(r"\s+", "", value) for name, value in text.items()}
    required = {
        "crates/dfmcp-adapter/src/lib.rs": ["pubmodworkshop_construction;"],
        MCP + "lib.rs": ["pubmodworkshop_construction_server;"],
        MCP + "bin/dfmcp-workshop-v1-20-dev-server.rs": ["dfmcp_mcp::workshop_construction_server::run_stdio();"],
        ADAPTER + "mod.rs": ["pubmodacknowledgement;", "pubmodjournal;", "pubmodrpc;"],
        ADAPTER + "rpc.rs": ["self.prepared.take().as_ref()==Some(permit.plan())"],
        MCP + "workshop_construction_server/runtime.rs": ["open_private_acknowledgements(", "acks.latest()", "self.withdraw();"],
        MCP + "workshop_construction_server/presentation.rs": ["spine(operation,session,request)", "canonical_world_anchor_available", "\"fortress_commit\"", "\"fortress_plan\"", "\"fortress_observe\""],
    }
    for name, tokens in required.items():
        if any(token not in compact[name] for token in tokens):
            raise ValueError(f"required workshop consumer boundary missing: {name}")
    server = text[MCP + "workshop_construction_server.rs"]
    handlers = re.findall(r"pub fn (fortress_\w+)\(", server)
    if len(handlers) != len(TOOLS) or set(handlers) != set(TOOLS):
        raise ValueError("registered workshop handler list differs")
    for handler in ("OpenSession", "Observe", "Query", "Plan", "Commit", "Wait", "Cancel", "Checkpoint", "Restore", "Explain", "Doctor"):
        if f".tool(Fortress{handler})" not in compact[MCP + "workshop_construction_server.rs"]:
            raise ValueError("workshop handler is not registered")
    journal = compact[ADAPTER + "journal.rs"]
    dispatch = journal.index("entry.state=WorkshopState::DispatchStarted;")
    writer = journal.index("source.commit(", dispatch)
    acknowledge = journal.find("guard.acknowledge(self.pin(),", dispatch, writer)
    if acknowledge < 0:
        raise ValueError("independent acknowledgement missing before native writer")
    if "generation>=self.generation" not in compact[ADAPTER + "mod.rs"]:
        raise ValueError("workshop source fencing missing")
    for wrapper in ("scripts/verify.sh", "scripts/qualify_local.sh"):
        if any(name not in text[wrapper] for name in ("check_workshop_construction.py", "test_workshop_source_inventory.py", "test_workshop_native.py --mutations", "tests/stdio/workshop.py")):
            raise ValueError("workshop static/component/syntax gates missing from mandatory wrapper")
    native = ast.parse(text["scripts/test_workshop_native.py"])
    constants = {}
    inputs = None
    mutants = None
    for node in native.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            name = node.targets[0].id
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                constants[name] = node.value.value
            if name == "INPUTS" and isinstance(node.value, ast.List):
                inputs = [constants[value.id] if isinstance(value, ast.Name) else ast.literal_eval(value) for value in node.value.elts]
            if name == "MUTATIONS" and isinstance(node.value, ast.Dict):
                mutants = [ast.literal_eval(key) for key in node.value.keys]
    if inputs != contract["native_component_source_paths"] or not mutants or len(mutants) != 12 or len(set(mutants)) != 12:
        raise ValueError("native required inventory or individual mutant list differs")
    if not {"missing_worker_labor_validity", "missing_worker_job_availability"} <= set(mutants):
        raise ValueError("independent worker guards lack individual mutants")
    actual_counts = {
        "adapter_rust_cases": sum(text[ADAPTER + name].count("#[test]") for name in ("tests.rs", "journal_tests.rs", "rpc_tests.rs")),
        "mcp_unit_cases": text[MCP + "workshop_construction_server/tests.rs"].count("#[test]"),
        "rust_stdio_cases": text["crates/dfmcp-mcp/tests/workshop_construction_stdio.rs"].count("#[test]"),
        "python_stdio_cases": len(re.findall(r"^    def test_", text["tests/stdio/workshop.py"], re.MULTILINE)),
        "python_inventory_cases": len(re.findall(r"^    def test_", text["scripts/test_workshop_source_inventory.py"], re.MULTILINE)),
        "native_scenarios": len(re.findall(r'run\("[^"]+",', text["tests/native/workshop/handler.cpp"])),
        "compiled_mutants": len(mutants),
    }
    if any(contract["focused_test_contract"][key] != value for key, value in actual_counts.items()):
        raise ValueError(f"workshop exact registered test count differs: {actual_counts}")
    return {"status": "passed_static_only", "source_sha256": hashes, "rust_executed": False,
            "sdk_or_game_executed": False, "production_admitted": False}


if __name__ == "__main__":
    print(json.dumps(check(), sort_keys=True, indent=2))
