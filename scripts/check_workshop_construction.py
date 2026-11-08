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
TREE = "crates/dfmcp-adapter/src/tree_harvest/"
MCP = "crates/dfmcp-mcp/src/"
METHODS = ["Handshake", "ReadWorkshop", "PrepareWorkshop", "CommitWorkshop", "QueryWorkshop", "CancelWorkshop", "ReadWorkshopProgress"]
TOOLS = ["fortress_" + name for name in ("open_session", "observe", "query", "plan", "commit", "wait", "cancel", "checkpoint", "restore", "explain", "doctor")]
SOURCES = [
    "architecture/workshop_construction_v1_20.json", "bridge/common/retained_snapshot.h", "bridge/common/workshop_construction.h", "bridge/common/one_attempt_native.h",
    "bridge/dfhack-workshop-v1_20/CMakeLists.txt", "bridge/dfhack-workshop-v1_20/DfmcpWorkshopV1_20.proto",
    "bridge/dfhack-workshop-v1_20/dfmcp_workshop_v1_20.cpp", "crates/dfmcp-adapter/src/lib.rs",
    "crates/dfmcp-adapter/src/build_placement/rpc/codec.rs", "crates/dfmcp-adapter/src/build_placement/rpc/link.rs",
    *[ADAPTER + name for name in ("mod.rs", "acknowledgement.rs", "journal.rs", "journal_tests.rs", "rpc.rs", "rpc_tests.rs", "tests.rs")],
    ADAPTER + "production_origin.rs", ADAPTER + "production_origin_tests.rs",
    ADAPTER + "clock.rs", ADAPTER + "clock/tests.rs",
    ADAPTER + "tree_material.rs", ADAPTER + "tree_material_tests.rs",
    *[TREE + name for name in ("mod.rs", "journal.rs", "material_origin.rs", "rpc.rs", "tests.rs", "journal_tests.rs")],
    "tests/native/tree_harvest/vectors.json",
    MCP + "lib.rs", MCP + "workshop_construction_server.rs",
    *[MCP + "workshop_construction_server/" + name for name in ("runtime.rs", "presentation.rs", "tests.rs", "clock.rs", "clock_config_tests.rs", "clock_presentation_tests.rs", "material_selection.rs", "tree_material.rs", "tree_material_tests.rs")],
    MCP + "bin/dfmcp-workshop-v1-20-dev-server.rs", "crates/dfmcp-mcp/tests/workshop_construction_stdio.rs",
    *["tests/native/workshop/" + name for name in ("stubs.h", "handler.cpp", "vectors.json", "continuity_vectors.json")],
    "tests/stdio/workshop.py", "tests/stdio/workshop_tree_material.py", "tests/stdio/workshop_clock.py", "tests/stdio/tree_harvest.py",
    "scripts/read_stable_repository_file.py", "scripts/test_workshop_native.py",
    "scripts/test_workshop_source_inventory.py", "scripts/check_workshop_construction.py", "scripts/verify.sh",
    "scripts/qualify_local.sh", "docs/WORKSHOP_CONSTRUCTION_MCP.md", "docs/TREE_HARVEST_MCP.md",
    "docs/WORKSHOP_CLOCK.md",
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
        ADAPTER + "mod.rs": ["pubmodacknowledgement;", "pubmodjournal;", "pubmodrpc;", "modtree_material;", "WorkshopMaterialDiscovery,WorkshopMaterialOffer", "pubusejournal::production_origin;", "pubfnfrom_source_pins("],
        ADAPTER + "journal.rs": ["pubmodproduction_origin;"],
        ADAPTER + "acknowledgement.rs": ["fnverify_workshop_offline(", "matches!(self.protocol,Protocol::Workshop)&&self.scope.is_none()&&self.offline"],
        ADAPTER + "production_origin.rs": ["pubfnproduction_origin<", "self.production_floors(key,&mutwork)?", "acknowledgement.verify_workshop_offline(", "pubfnretain_recovery_floor(", "pubfnfence(&mutself)", "pubfnretain_source_manifest(", "self.high_tick=self.high_tick.max(progress.tick());", "ifgoal!=WorkshopGoal::Complete"],
        TREE + "journal.rs": ["modmaterial_origin;", "pubusematerial_origin::TreeMaterialOrigin;"],
        TREE + "material_origin.rs": ["pubfnmaterial_origin<", "acknowledgement.verify_tree(", "self.material_projection(&self.raw"],
        ADAPTER + "tree_material.rs": ["pubconstMAX_OFFERS:usize=64;", "Progress::decode(progress.bytes(),&origin.original().plan)?!=progress", "capture.tick()!=self.progress.tick"],
        ADAPTER + "rpc.rs": ["self.prepared.take().as_ref()==Some(permit.plan())"],
        MCP + "workshop_construction_server/runtime.rs": ["open_private_acknowledgements(", "acks.latest()", "self.withdraw();", "modtree_material;", "modmaterial_selection;", "self.tree_material=None;", "advance_authorization_tick"],
        MCP + "workshop_construction_server/tree_material.rs": ["WorkshopMode::Offline", "self.journal.material_origin(", "source.progress(", "self.high_tick=self.high_tick.max(progress.tick)", "self.verify(work)?;"],
        MCP + "workshop_construction_server/material_selection.rs": ["donor.qualify(handle,&capture,work)?;", "donor.verify(work)?;", "self.guard.selection=Some(selection);"],
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
        if any(name not in text[wrapper] for name in ("check_workshop_construction.py", "test_workshop_source_inventory.py", "test_workshop_native.py --mutations", "tests/stdio/workshop.py", "tests/stdio/workshop_tree_material.py", "tests/stdio/workshop_clock.py")):
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
        "clock_adapter_rust_cases": text[ADAPTER + "clock/tests.rs"].count("#[test]"),
        "tree_material_adapter_rust_cases": text[ADAPTER + "tree_material_tests.rs"].count("#[test]"),
        "production_origin_adapter_rust_cases": text[ADAPTER + "production_origin_tests.rs"].count("#[test]"),
        "mcp_unit_cases": text[MCP + "workshop_construction_server/tests.rs"].count("#[test]"),
        "clock_mcp_config_cases": text[MCP + "workshop_construction_server/clock_config_tests.rs"].count("#[test]"),
        "clock_mcp_presentation_cases": text[MCP + "workshop_construction_server/clock_presentation_tests.rs"].count("#[test]"),
        "clock_python_stdio_cases": len(re.findall(r"^    def test_", text["tests/stdio/workshop_clock.py"], re.MULTILINE)),
        "tree_material_mcp_unit_cases": text[MCP + "workshop_construction_server/tree_material_tests.rs"].count("#[test]"),
        "rust_stdio_cases": text["crates/dfmcp-mcp/tests/workshop_construction_stdio.rs"].count("#[test]"),
        "python_stdio_cases": len(re.findall(r"^    def test_", text["tests/stdio/workshop.py"], re.MULTILINE)),
        "tree_material_python_stdio_cases": len(re.findall(r"^    def test_", text["tests/stdio/workshop_tree_material.py"], re.MULTILINE)),
        "python_inventory_cases": len(re.findall(r"^    def test_", text["scripts/test_workshop_source_inventory.py"], re.MULTILINE)),
        "native_scenarios": len(re.findall(r'run\("[^"]+",', text["tests/native/workshop/handler.cpp"])),
        "compiled_mutants": len(mutants),
    }
    if any(contract["focused_test_contract"][key] != value for key, value in actual_counts.items()):
        raise ValueError(f"workshop exact registered test count differs: {actual_counts}")
    expected_production_origin = {
        "api": "workshop_construction::production_origin",
        "origin": "exact original Placed receipt and independently acknowledged private Offline Workshop custody",
        "historical_completion_required": False,
        "fresh_complete_required": True,
        "donor_reverified_around_native_reads": True,
        "donor_writes": False,
        "native_methods_added": False,
        "public_tools_added": False,
        "canonical_world_fact": False,
        "original_descriptor_immutable": True,
        "owner_cloneable": False,
        "consumed_floors": ["tick", "native_generation", "next_building_id", "sticky_fence"],
        "recovery_floor_custody": "externally retained by the importing host; not a local durable acknowledgement of fresh reads",
        "connection_failure": "host invokes one-way fence before any future qualification",
        "decoded_manifest_consumed_before_refresh_preconditions": True,
        "read_reservations_bytes": {"import": 100663296, "verify": 25165824, "refresh": 67108864},
        "association_max_bytes": 65536,
        "association_create_authority": ["Query", "Plan"],
        "association_reopen_mode": "Offline",
        "combined_host_integration": "tracked separately from this embedding API",
        "manager_material_or_causal_production_proof": False,
    }
    if contract.get("production_origin_adapter") != expected_production_origin:
        raise ValueError("Workshop production origin adapter contract differs")
    if any(token in compact[ADAPTER + "production_origin.rs"] for token in
           ("source.prepare(", "source.commit(", "source.cancel(", "journal.sample(")):
        raise ValueError("Workshop production importer cannot mutate or advance donor custody")
    expected_tree_material_adapter = {
        "source_profile": "tree_harvest/1.21",
        "origin": "exact original designation and independently acknowledged private Offline custody",
        "donor_writes": False,
        "arbitrary_native_item_selector": False,
        "maximum_offers": 64,
        "complete_supply_required": True,
        "origin_goal_completion_required": False,
        "original_deadline_renewed": False,
        "causal_provenance_known": False,
        "handle_binding": ["original custody", "complete fresh Tree progress", "session", "workshop center", "item"],
        "independent_workshop_qualification": ["fortress", "map dimensions", "same paused game tick", "selected log position and material", "workshop eligibility"],
        "generations_independently_pinned": True,
        "selection_latch": "original Workshop Intent capture",
        "recovery_rediscovery": False,
    }
    if contract.get("tree_material_adapter") != expected_tree_material_adapter:
        raise ValueError("Tree material adapter contract differs")
    expected_tree_material_consumer = {
        "schema": "dfmcp.workshop-tree-material-mcp-development/1",
        "opt_in": "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL=1",
        "read_credential": "DFMCP_TREE_HARVEST_TOKEN",
        "operator_item_id": False,
        "donor_config_fields": ["directory", "endpoint", "generation", "root", "scope", "read_scope", "acknowledged"],
        "read_scope": "complete loaded map",
        "donor_mode": "offline exclusive private custody",
        "donor_reverified_around_native_reads": True,
        "original_tree_mutation_authority": False,
        "native_protocols_unchanged": True,
        "plan_kind": "carpenters_from_tree_supply",
        "plan_fields": ["kind", "expected_observation", "material"],
        "handle_prefix": "tree-workshop-material:",
        "maximum_offers": 64,
        "default_request_bytes": 268435456,
        "legacy_default_request_bytes": 134217728,
        "host_selection_allowed": "operator scope, protected regions and footprint only; no native eligibility claim",
        "consumed_authority_tick_survives_failure": True,
        "donor_released_after": "immutable Workshop Intent and native preparation",
        "recovery_material_source": "original Workshop Intent capture only",
        "recovery_tree_files_opened": False,
        "recovery_tree_credential_read": False,
        "recovery_tree_native_calls": False,
        "harvest_goal_completion_proven": False,
        "causal_provenance_known": False,
    }
    if contract.get("tree_material_consumer") != expected_tree_material_consumer:
        raise ValueError("Tree material MCP consumer contract differs")
    expected_review_budget = {
        "bead": "dfmcp-reality-wiring-proof-4bf.14.3",
        "default_wall_millis": 60000, "maximum_wall_millis": 60000,
        "connection_deadline_renewal": False,
        "pre_dispatch_expiry": "reject before durable dispatch; consume local permission; retain original preparation custody for reconciliation",
        "after_dispatch_failure": "indeterminate original effect; no automatic retry",
    }
    if contract.get("prepared_review_budget") != expected_review_budget:
        raise ValueError("prepared review budget contract differs")
    if "unwrap_or(60000)" not in text["crates/dfmcp-mcp/src/workshop_construction_server.rs"]:
        raise ValueError("prepared review default differs")
    if journal.index("ifletErr(e)=self.fresh(source,&entry.plan,&mutwork,guard)") >= dispatch:
        raise ValueError("fresh read must precede durable dispatch")
    return {"status": "passed_static_only", "source_sha256": hashes, "rust_executed": False,
            "sdk_or_game_executed": False, "production_admitted": False}


if __name__ == "__main__":
    print(json.dumps(check(), sort_keys=True, indent=2))
