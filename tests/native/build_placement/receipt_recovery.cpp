// Beads: df-dfhack-bridge-plane-c-pic.4/.5. Real RPC handlers, explicit SDK/message doubles.
#include "stubs.h"
#include "../../../bridge/dfhack-build-v1_19/dfmcp_build_v1_19.cpp"
#include <iostream>

namespace {
unsigned checks = 0, scenarios = 0;
#define CHECK(x) do { ++checks; if (!(x)) throw std::runtime_error("check failed: " #x); } while (false)
df::world world;
color_ostream output;
using Rpc = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
void cleanup() {
    world.buildings.all.clear(); world.buildings.other.ANY_ZONE.clear();
    world.items.all.clear(); world.jobs.list.next = nullptr;
    while (!df::live_buildings.empty()) delete *df::live_buildings.begin();
    mock::jobs.clear(); mock::links.clear(); mock::general_refs.clear();
    mock::specific_refs.clear(); mock::item_refs.clear(); mock::items.clear(); mock::blocks.clear();
}
void reset(unsigned kind = 1) {
    cleanup(); engine = bp::Engine(7);
    mock::loaded = mock::mapped = mock::fortress = mock::paused = mock::initialized = true;
    mock::unknown = mock::reply_failure = mock::oversized_reply = mock::oversized_request = false;
    mock::missing_block = false; mock::tick = 12345; mock::next_building = 70; mock::next_job = 90;
    mock::after_construct = {}; mock::during_verify = {}; mock::after_allocation = {};
    mock::write_fault = mock::WriteFault::None;
    df::global::world = &world; df::global::plotinfo = &world;
    for (int y = 0; y < 2; ++y) for (int x = 0; x < 2; ++x)
        mock::blocks[{x, y, 2}] = std::make_unique<df::map_block>();
    mock::items.push_back(std::make_unique<df::item>());
    mock::items.back()->type = static_cast<df::item_type>(kind);
    world.items.all.push_back(mock::items.back().get());
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", "1", 1);
    setenv("DFMCP_BUILD_ALLOW_PLACE", "1", 1);
    setenv("DFMCP_BUILD_TOKEN", std::string(32, 's').c_str(), 1);
    mock::game_access = mock::writer_calls = mock::allocator_calls = mock::holder_calls = 0;
}
wire::Reply call(Rpc rpc, wire::Request q, unsigned mask) {
    q.mask = mask; wire::Reply r; CHECK(rpc(output, &q, &r) == CR_OK); return r;
}
wire::Request prepare(unsigned kind = 1) {
    wire::Request q; q.kk = kind;
    auto r = call(ReadPlacement, q, 31); CHECK(r.accepted);
    q.witness = dfmcp_snapshot::sha256(r.observation);
    q.plan = bp::plan_digest(selection(&q), q.witness);
    r = call(PreparePlacement, q, 255); CHECK(r.accepted && !r.replayed);
    q.token = bp::token_for(q.key, q.plan); return q;
}
const bp::Record &retained(const wire::Request &q) {
    const auto *r = engine.query(q.key, q.plan); CHECK(r); return *r;
}
wire::Request uncertain(unsigned kind = 1, bool verify_fault = false) {
    reset(kind); auto q = prepare(kind);
    if (verify_fault) mock::during_verify = [](df::job *) { throw std::runtime_error("verification unavailable"); };
    else mock::after_construct = [](df::building *, df::job *, df::item *) {
        mock::missing_block = true; mock::missing_location = {0, 0, 2};
    };
    const auto r = call(CommitPlacement, q, 416);
    CHECK(r.accepted && r.unresolved && retained(q).phase == bp::Phase::Indeterminate);
    CHECK(mock::writer_calls == 1);
    mock::missing_block = false; mock::after_construct = {}; mock::during_verify = {};
    return q;
}
void remains_unknown(const wire::Request &q, const std::string &before) {
    const auto r = call(QueryPlacement, q, 160);
    CHECK(r.accepted && r.unresolved && r.effect == before);
    CHECK(engine.unresolved() && mock::writer_calls == 1);
    CHECK(retained(q).phase == bp::Phase::Indeterminate);
}
void recover_all_kinds() {
    for (unsigned kind = 1; kind <= 3; ++kind) {
        reset(kind); auto direct = prepare(kind);
        const auto expected = call(CommitPlacement, direct, 416).effect;
        for (bool verify_fault : {false, true}) {
            const auto q = uncertain(kind, verify_fault); const auto before = retained(q).encode();
            setenv("DFMCP_BUILD_ALLOW_PLACE", "0", 1);
            const auto allocations = mock::allocator_calls;
            const auto r = call(QueryPlacement, q, 160);
            CHECK(r.accepted && !r.unresolved && r.records == 1 && r.mask == 26);
            CHECK(r.effect == expected && r.effect != before);
            CHECK(retained(q).phase == bp::Phase::Placed && !engine.unresolved());
            CHECK(mock::writer_calls == 1 && mock::allocator_calls == allocations);
            const auto accesses = mock::game_access;
            mock::mapped = false;
            CHECK(call(QueryPlacement, q, 160).effect == expected);
            CHECK(mock::game_access == accesses && mock::writer_calls == 1);
        }
    }
}
void no_effect_or_partial_links_never_resolve() {
    for (auto fault : {mock::WriteFault::FalseBefore, mock::WriteFault::ThrowBefore,
                      mock::WriteFault::BuildingOnly, mock::WriteFault::JobOnly}) {
        reset(); const auto q = prepare(); mock::write_fault = fault;
        const auto r = call(CommitPlacement, q, 416); CHECK(r.accepted && r.unresolved);
        for (unsigned n = 0; n < 3; ++n) remains_unknown(q, r.effect);
        CHECK(call(CancelPlacement, q, 416).effect == r.effect);
        CHECK(call(CommitPlacement, q, 416).effect == r.effect);
        CHECK(mock::writer_calls == 1);
    }
}
void source_tick_and_intervention_fences() {
    for (unsigned change = 0; change < 5; ++change) {
        const auto q = uncertain(); const auto before = retained(q).encode();
        if (change == 0) engine.change_source();
        else if (change == 1) engine.interrupt();
        else if (change == 2) ++mock::tick;
        else if (change == 3) mock::paused = false;
        else mock::mapped = false;
        const auto accesses = mock::game_access;
        remains_unknown(q, before);
        if (change < 2) CHECK(mock::game_access == accesses);
    }
}
void exact_relationships_and_final_read_are_required() {
    for (unsigned change = 0; change < 7; ++change) {
        const auto q = uncertain(); const auto before = retained(q).encode();
        auto *job = world.buildings.all.front()->jobs.front();
        if (change == 0) job->items.front()->role = df::job_role_type::Reagent;
        else if (change == 1) job->general_refs.front()->holder = nullptr;
        else if (change == 2) job->flags.bits.suspend = true;
        else if (change == 3) world.buildings.all.front()->stage = 1;
        else if (change == 4) mock::items.front()->specific_refs.clear();
        else if (change == 5) mock::during_verify = [](df::job *) { ++mock::tick; };
        else mock::during_verify = [](df::job *) { throw std::runtime_error("unavailable"); };
        remains_unknown(q, before);
    }
}
void entry_authority_and_shape_precede_recovery_reads() {
    const auto q = uncertain(); const auto before = retained(q).encode();
    for (unsigned mask = 0; mask < 512; ++mask) {
        if (mask == 160) continue;
        const auto accesses = mock::game_access;
        const auto r = call(QueryPlacement, q, mask);
        CHECK(!r.accepted && r.code == 3 && r.effect.empty() && r.mask == 0);
        CHECK(mock::game_access == accesses && retained(q).encode() == before);
    }
    for (unsigned gate = 0; gate < 4; ++gate) {
        auto request = q;
        if (gate == 0) request.bearer[0] = 'x';
        else if (gate == 1) setenv("DFMCP_BUILD_TOKEN", std::string(32, 'x').c_str(), 1);
        else if (gate == 2) setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", "0", 1);
        else setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "", 1);
        const auto accesses = mock::game_access;
        const auto r = call(QueryPlacement, request, 160);
        CHECK(!r.accepted && r.code == 1 && r.effect.empty());
        CHECK(mock::game_access == accesses && retained(q).encode() == before);
        setenv("DFMCP_BUILD_TOKEN", std::string(32, 's').c_str(), 1);
        setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", "1", 1);
        unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    }
    auto wrong = q; wrong.plan[0] ^= 1;
    const auto accesses = mock::game_access;
    CHECK(!call(QueryPlacement, wrong, 160).accepted);
    wrong = q; wrong.key = "absent";
    const auto absent = call(QueryPlacement, wrong, 160);
    CHECK(absent.accepted && absent.unresolved && absent.effect.empty());
    CHECK(mock::game_access == accesses && mock::writer_calls == 1);
}
void lost_recovery_reply_replays_without_reread_or_redispatch() {
    const auto q = uncertain(); mock::reply_failure = true;
    const auto lost = call(QueryPlacement, q, 160);
    CHECK(!lost.accepted && lost.effect.empty());
    CHECK(retained(q).phase == bp::Phase::Placed && !engine.unresolved());
    const auto expected = retained(q).encode();
    const auto accesses_before = mock::game_access;
    const auto recovered = call(QueryPlacement, q, 160);
    CHECK(recovered.accepted && !recovered.unresolved && recovered.effect == expected);
    CHECK(mock::game_access == accesses_before && mock::writer_calls == 1);
}
}
int main() {
    try {
        for (auto test : {recover_all_kinds, no_effect_or_partial_links_never_resolve,
             source_tick_and_intervention_fences, exact_relationships_and_final_read_are_required,
             entry_authority_and_shape_precede_recovery_reads,
             lost_recovery_reply_replays_without_reread_or_redispatch}) {
            test(); ++scenarios;
        }
        cleanup();
        std::cout << "{\"scenarios\":" << scenarios << ",\"assertions\":" << checks
                  << ",\"actual_rpc_handler\":true,\"sdk_doubles_only\":true}\n";
        return 0;
    } catch (const std::exception &e) {
        cleanup(); std::cerr << e.what() << '\n'; return 1;
    }
}
