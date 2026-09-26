#include "stubs.h"
#include "../../../bridge/dfhack-build-v1_19/dfmcp_build_v1_19.cpp"
#include <iostream>

namespace {
std::size_t checks = 0;
std::vector<std::string> groups;
std::string current_group;
#define CHECK(condition) do { ++checks; if (!(condition)) throw std::runtime_error( \
    "check failed: " + current_group + ":" + std::to_string(__LINE__) + ": " #condition); } while (false)
color_ostream output;
df::world world;
using Rpc = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
constexpr unsigned READ = 31, PREPARE = 255, COMMIT = 416, QUERY = 160;

void cleanup() {
    world.buildings.other.ANY_ZONE.clear(); world.buildings.all.clear(); world.items.all.clear(); world.jobs.list.next = nullptr;
    while (!df::live_buildings.empty()) delete *df::live_buildings.begin();
    mock::jobs.clear(); mock::links.clear(); mock::general_refs.clear();
    mock::specific_refs.clear(); mock::item_refs.clear(); mock::items.clear(); mock::blocks.clear();
}
void reset() {
    cleanup(); engine = bp::Engine(7);
    mock::loaded = mock::mapped = mock::fortress = mock::paused = mock::initialized = true;
    mock::unknown = mock::reply_failure = mock::oversized_reply = mock::oversized_request = false;
    mock::free_tile = mock::supported = mock::free_arguments_safe = true;
    mock::allocation_failure = mock::resize_failure = mock::missing_block = mock::invalid_dfhack_version = false;
    mock::year = 0; mock::tick = 12345; mock::site = 1;
    mock::size_x = mock::size_y = 64; mock::size_z = 8;
    mock::folder = "region1"; mock::df_version = "test-df"; mock::dfhack_version = "test-dfhack";
    mock::next_building = 70; mock::next_job = 90;
    mock::write_fault = mock::WriteFault::None;
    mock::after_construct = {}; mock::during_verify = {}; mock::after_allocation = {};
    df::global::world = &world; df::global::plotinfo = &world;
    df::global::building_next_id = &mock::next_building; df::global::job_next_id = &mock::next_job;
    Core::getInstance().vinfo = std::make_shared<VersionInfo>();
    for (int y = 0; y < 2; ++y) for (int x = 0; x < 2; ++x)
        mock::blocks[{x, y, 2}] = std::make_unique<df::map_block>();
    mock::items.push_back(std::make_unique<df::item>()); world.items.all.push_back(mock::items.back().get());
    mock::blocks.at({0, 0, 2})->occupancy[10][11].bits.item = 1;
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", "1", 1);
    setenv("DFMCP_BUILD_ALLOW_PLACE", "1", 1);
    setenv("DFMCP_BUILD_TOKEN", std::string(32, 's').c_str(), 1);
    mock::game_access = mock::item_attribute_reads = mock::writer_calls = mock::allocator_calls = 0;
    mock::support_calls = mock::free_calls = mock::holder_calls = 0;
}
df::map_block &block(int x = 15, int y = 15) { return *mock::blocks.at({x / 16, y / 16, 2}); }
df::item &item() { return *mock::items.front(); }
wire::Reply call(Rpc method, const wire::Request &request) {
    wire::Reply reply; CHECK(method(output, &request, &reply) == CR_OK); return reply;
}
void denied(const wire::Reply &reply, unsigned code) {
    CHECK(!reply.accepted); CHECK(reply.code == code); CHECK(reply.mask == 0);
    CHECK(reply.effect.empty() && reply.observation.empty());
    CHECK(reply.generation == 0 && reply.df_version.empty() && reply.dfhack_version.empty());
    CHECK(reply.major == 1 && reply.minor == 19 && reply.nonce.size() <= 64);
}
wire::Reply read(wire::Request request = {}) { request.mask = READ; return call(ReadPlacement, request); }
wire::Request candidate(wire::Request request = {}) {
    const auto observed = read(request); CHECK(observed.accepted); CHECK(!observed.observation.empty());
    request.witness = dfmcp_snapshot::sha256(observed.observation);
    request.plan = bp::plan_digest(selection(&request), request.witness); request.mask = PREPARE;
    return request;
}
wire::Request prepared(wire::Request request = {}) {
    request = candidate(request); const auto reply = call(PreparePlacement, request);
    CHECK(reply.accepted); CHECK(!reply.replayed && !reply.unresolved); CHECK(reply.records == engine.size());
    const auto *record = engine.query(request.key, request.plan); CHECK(record);
    CHECK(record->phase == bp::Phase::Prepared); request.token = record->token;
    return request;
}
wire::Reply commit(wire::Request request) { request.mask = COMMIT; return call(CommitPlacement, request); }
wire::Reply query(wire::Request request) { request.mask = QUERY; return call(QueryPlacement, request); }
wire::Reply cancel(wire::Request request) { request.mask = COMMIT; return call(CancelPlacement, request); }
const bp::Record &retained(const wire::Request &request) {
    const auto *record = engine.query(request.key, request.plan); CHECK(record); return *record;
}
void ineligible() {
    const auto q = candidate(); denied(call(PreparePlacement, q), 4);
    CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0 && engine.size() == 0);
}
df::building *existing(int id = 1, int x = 25, int y = 25, df::building_type type = df::building_type::Bed) {
    auto *building = new df::building_actual(); building->id = id; building->type = type;
    building->x1 = building->x2 = building->centerx = x; building->y1 = building->y2 = building->centery = y;
    building->z = 2; world.buildings.all.push_back(building); return building;
}
void indeterminate(const wire::Request &q, const wire::Reply &r) {
    CHECK(r.accepted && r.unresolved); CHECK(retained(q).phase == bp::Phase::Indeterminate);
    CHECK(retained(q).reason == bp::Reason::NativeFailure); CHECK(!retained(q).after && !retained(q).insertion);
    CHECK(engine.unresolved()); CHECK(r.effect == retained(q).encode());
}
void run(const char *name, const std::function<void()> &body) {
    current_group = name; reset(); body(); groups.emplace_back(name);
}

void rpc_manifest() {
    std::vector<PluginCommand> commands; CHECK(plugin_init(output, commands) == CR_OK); CHECK(commands.empty());
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));
    const std::array<std::string, 6> names{"Handshake", "ReadPlacement", "PreparePlacement",
        "CommitPlacement", "QueryPlacement", "CancelPlacement"};
    CHECK(service->methods.size() == names.size());
    for (std::size_t i = 0; i < names.size(); ++i) {
        CHECK(service->methods[i].first == names[i]); CHECK(service->methods[i].second == 0);
    }
    const auto reply = call(Handshake, {});
    CHECK(reply.accepted && reply.code == 0 && reply.generation == 7);
    CHECK(reply.df_version == "test-df" && reply.dfhack_version == "test-dfhack");
    CHECK(reply.mask == 24 && !reply.unresolved && reply.records == 0); CHECK(mock::game_access == 0);
}
void environment_gates() {
    for (const char *value : {"", "0", "true", "yes", "01", "1 "}) {
        setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", value, 1);
        denied(call(Handshake, {}), 1); CHECK(mock::game_access == 0);
    }
    unsetenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19"); denied(call(Handshake, {}), 1);
    CHECK(mock::game_access == 0 && mock::writer_calls == 0);
}
void admitted_process_isolation() {
    for (const char *value : {"", "1.0", "1.19"}) {
        setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", value, 1);
        denied(call(Handshake, {}), 1); CHECK(mock::game_access == 0);
    }
    reset(); const auto q = prepared();
    mock::after_allocation = [] { setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "", 1); };
    indeterminate(q, commit(q)); CHECK(mock::writer_calls == 0 && world.buildings.all.empty());
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL"); CHECK(query(q).accepted && query(q).unresolved);
}
void bearer_bounds_and_equality() {
    for (unsigned length : {0u, 1u, 31u, 257u}) {
        wire::Request q; q.bearer.assign(length, 's'); denied(call(Handshake, q), 1);
        setenv("DFMCP_BUILD_TOKEN", q.bearer.c_str(), 1); denied(call(Handshake, q), 1);
        setenv("DFMCP_BUILD_TOKEN", std::string(32, 's').c_str(), 1);
    }
    for (unsigned length : {32u, 64u, 256u}) {
        wire::Request q; q.bearer.assign(length, 's'); setenv("DFMCP_BUILD_TOKEN", q.bearer.c_str(), 1);
        CHECK(call(Handshake, q).accepted);
        q.bearer[length - 1] = 'x'; denied(call(Handshake, q), 1);
        q.bearer = std::string(length, 's') + 's'; denied(call(Handshake, q), 1);
    }
    unsetenv("DFMCP_BUILD_TOKEN"); denied(call(Handshake, {}), 1); CHECK(mock::game_access == 0);
}
void protocol_and_nonce() {
    for (unsigned version : {0u, 1u, 18u, 20u, UINT32_MAX}) {
        wire::Request q; q.minor = version; denied(call(Handshake, q), 2);
    }
    wire::Request q; q.major = 2; denied(call(Handshake, q), 2);
    for (unsigned length : {0u, 15u, 65u, 1024u}) {
        q = {}; q.nonce.assign(length, 'n'); denied(call(Handshake, q), 3);
    }
    q = {}; q.nonce = std::string(64, '\0'); CHECK(call(Handshake, q).accepted);
    CHECK(mock::game_access == 0);
}
void message_initialization_and_unknown_fields() {
    mock::unknown = true; denied(call(Handshake, {}), 3); mock::unknown = false;
    mock::initialized = false; denied(call(Handshake, {}), 3); mock::initialized = true;
    mock::oversized_request = true; denied(call(Handshake, {}), 3);
    CHECK(mock::game_access == 0 && mock::writer_calls == 0);
}
void read_shapes() {
    for (unsigned mask = 0; mask < 512; ++mask) {
        wire::Request q; q.mask = mask; mock::game_access = 0;
        const auto h = call(Handshake, q); CHECK(h.accepted == (mask == 0)); CHECK(mock::game_access == 0);
        const auto r = call(ReadPlacement, q); CHECK(r.accepted == (mask == READ));
        if (mask != READ) { denied(r, 3); CHECK(mock::game_access == 0); }
    }
    CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
}
void effect_shapes() {
    const auto plan = prepared();
    for (const auto &[rpc, allowed] : std::array<std::pair<Rpc, unsigned>, 4>{{
        {PreparePlacement, PREPARE}, {CommitPlacement, COMMIT}, {QueryPlacement, QUERY}, {CancelPlacement, COMMIT}}}) {
        for (unsigned mask = 0; mask < 512; ++mask) if (mask != allowed) {
            auto q = plan; q.mask = mask; mock::game_access = 0; denied(call(rpc, q), 3); CHECK(mock::game_access == 0);
        }
    }
    CHECK(retained(plan).phase == bp::Phase::Prepared && mock::writer_calls == 0);
}
void placement_authority_and_recovery() {
    const auto q = prepared();
    for (const char *value : {"", "0", "true", "01", "1 "}) {
        setenv("DFMCP_BUILD_ALLOW_PLACE", value, 1); mock::game_access = 0;
        denied(call(PreparePlacement, q), 1); denied(commit(q), 1); CHECK(mock::game_access == 0);
    }
    unsetenv("DFMCP_BUILD_ALLOW_PLACE"); CHECK(read().accepted); CHECK(query(q).accepted);
    mock::game_access = 0; const auto r = cancel(q); CHECK(r.accepted); CHECK(mock::game_access == 0);
    CHECK(retained(q).phase == bp::Phase::Cancelled && mock::writer_calls == 0);
}
void manifest_validation() {
    for (int fault = 0; fault < 6; ++fault) {
        reset();
        if (fault == 0) Core::getInstance().vinfo.reset();
        if (fault == 1) mock::df_version.clear();
        if (fault == 2) mock::df_version.assign(129, 'd');
        if (fault == 3) mock::dfhack_version.assign("\xff", 1);
        if (fault == 4) mock::df_version = std::string("\xed\xa0\x80", 3);
        if (fault == 5) mock::invalid_dfhack_version = true;
        denied(call(Handshake, {}), 5); CHECK(mock::game_access == 0);
    }
}
void bounded_reply_and_clearing() {
    auto q = prepared(); mock::oversized_reply = true; denied(query(q), 5);
    mock::oversized_reply = false; auto r = query(q); CHECK(r.accepted && !r.effect.empty());
    q.mask = 0; q.bearer[0] = 'x'; CHECK(Handshake(output, &q, &r) == CR_OK); denied(r, 1);
    CHECK(r.records == 0 && !r.unresolved); CHECK(mock::writer_calls == 0);
}
void unavailable_world() {
    for (int fault = 0; fault < 4; ++fault) {
        reset();
        if (fault == 0) mock::loaded = false;
        if (fault == 1) mock::mapped = false;
        if (fault == 2) mock::fortress = false;
        if (fault == 3) df::global::world = nullptr;
        denied(read(), 4); CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
    }
}
void malformed_world_identity() {
    for (int fault = 0; fault < 10; ++fault) {
        reset();
        switch (fault) {
            case 0: mock::year = -1; break; case 1: mock::year = std::int64_t{UINT32_MAX} + 1; break;
            case 2: mock::tick = 403200; break; case 3: mock::site = -1; break;
            case 4: mock::folder.clear(); break; case 5: mock::folder.assign(513, 'x'); break;
            case 6: df::global::job_next_id = nullptr; break; case 7: mock::next_job = -1; break;
            case 8: df::global::building_next_id = nullptr; break; case 9: mock::next_building = -1; break;
        }
        denied(read(), 5); CHECK(mock::writer_calls == 0);
    }
}
void bounded_selection() {
    for (unsigned kind : {0u, 4u, 256u, UINT32_MAX}) {
        wire::Request q; q.kk = kind; mock::game_access = 0; denied(read(q), 3); CHECK(mock::game_access == 0);
    }
    for (int fault = 0; fault < 8; ++fault) {
        wire::Request q;
        switch (fault) {
            case 0: q.item = INT32_MAX; break; case 1: q.xx = 0; break; case 2: q.yy = 0; break;
            case 3: q.xx = 32767; break; case 4: q.zz = 32768; break;
            case 5: q.xx = 63; break; case 6: q.yy = 63; break; case 7: q.zz = 8; break;
        }
        denied(read(q), fault < 5 ? 3 : 4);
    }
    for (int value : {-1, 0, 32769}) { mock::size_x = value; denied(read(), 4); }
    CHECK(mock::writer_calls == 0);
}
void hidden_terrain_redaction() {
    block(14, 14).designation[14][14].bits.hidden = 1;
    const auto before = read(); CHECK(before.accepted); CHECK(mock::free_calls == 0 && mock::support_calls == 0);
    block(14, 14).tiletype[14][14] = df::tiletype::Bad;
    block(14, 14).occupancy[14][14].whole = UINT32_MAX;
    block(14, 14).designation[14][14].bits.dig = df::tile_dig_designation::Channel;
    block(14, 14).designation[14][14].bits.flow_size = 7;
    const auto after = read(); CHECK(after.accepted && after.observation == before.observation);
    ineligible(); CHECK(mock::free_calls == 0 && mock::support_calls == 0);
}
void missing_terrain_refusal() {
    mock::missing_block = true; mock::missing_location = {1, 1, 2};
    const auto r = read(); CHECK(r.accepted); ineligible();
    CHECK(mock::free_calls == 0 && mock::support_calls == 0);
}
void hidden_item_redaction() {
    for (int mechanism = 0; mechanism < 2; ++mechanism) {
        reset();
        if (mechanism == 0) item().flags.bits.hidden = 1;
        else block(10, 11).designation[10][11].bits.hidden = 1;
        const auto before = read(); CHECK(before.accepted && mock::item_attribute_reads == 0);
        item().material = -999; item().quality = -1; item().type = df::item_type::INVALID;
        item().flags2.whole = UINT32_MAX; item().general_refs.push_back(nullptr);
        const auto after = read(); CHECK(after.accepted && after.observation == before.observation);
        CHECK(mock::item_attribute_reads == 0); ineligible();
    }
}
void unavailable_item_selection() {
    item().id = 43; ineligible(); reset(); item().pos.x = -1; ineligible();
    reset(); item().pos.z = 8; denied(read(), 5);
    reset(); item().flags2.whole = 1; denied(read(), 5);
    reset(); item().quality = -1; denied(read(), 5);
    reset(); item().type = df::item_type::INVALID; denied(read(), 5);
    CHECK(mock::writer_calls == 0);
}
void target_floor_policy() {
    for (auto type : {df::tiletype::Wall, df::tiletype::Empty, df::tiletype::Ramp, df::tiletype::RampTop,
        df::tiletype::StairUp, df::tiletype::StairDown, df::tiletype::StairUpDown, df::tiletype::Unclassified}) {
        reset(); block().tiletype[15][15] = type; ineligible();
    }
    reset(); mock::paused = false; ineligible(); reset(); mock::free_tile = false; ineligible();
    reset(); mock::supported = false; ineligible(); reset(); block().designation[15][15].bits.pile = 1; ineligible();
    reset(); block().designation[15][15].bits.smooth = 1; ineligible();
}
void target_occupancy_and_designation_policy() {
    for (int flag = 0; flag < 8; ++flag) {
        reset(); auto &b = block();
        switch (flag) {
            case 0: b.occupancy[15][15].bits.building = df::tile_building_occ::Planned; break;
            case 1: b.occupancy[15][15].bits.unit = 1; break;
            case 2: b.occupancy[15][15].bits.unit_grounded = 1; break;
            case 3: b.occupancy[15][15].bits.item = 1; break;
            case 4: b.occupancy[15][15].bits.other = 1; break;
            case 5: b.designation[15][15].bits.hidden = 1; break;
            case 6: b.designation[15][15].bits.flow_size = 1; break;
            case 7: b.designation[15][15].bits.dig = df::tile_dig_designation::Default; break;
        }
        ineligible();
    }
}
void neighborhood_eligibility() {
    for (int y = 14; y <= 16; ++y) for (int x = 14; x <= 16; ++x) if (x != 15 || y != 15) {
        reset(); block(x, y).designation[x & 15][y & 15].bits.flow_size = 1; ineligible();
    }
    reset();
    for (const auto &[x, y] : std::array<std::pair<int, int>, 4>{{{14,15}, {16,15}, {15,14}, {15,16}}})
        block(x, y).tiletype[x & 15][y & 15] = df::tiletype::Wall;
    ineligible(); block(14, 15).tiletype[14][15] = df::tiletype::Floor;
    const auto q = prepared(); CHECK(commit(q).accepted); CHECK(retained(q).phase == bp::Phase::Placed);
}
void item_availability_policy() {
    for (int fault = 0; fault < 12; ++fault) {
        reset();
        switch (fault) {
            case 0: item().flags.bits.on_ground = false; break; case 1: item().flags.bits.in_job = true; break;
            case 2: item().flags.bits.forbid = true; break; case 3: item().flags.bits.removed = true; break;
            case 4: item().flags.bits.in_inventory = true; break; case 5: item().flags.bits.in_building = true; break;
            case 6: item().wear = 1; break; case 7: item().material = -1; break;
            case 8: item().type = df::item_type::CHAIR; break;
            case 9: block(10, 11).designation[10][11].bits.flow_size = 1; break;
            case 10: block(10, 11).tiletype[10][11] = df::tiletype::Wall; break;
            case 11: block(10, 11).occupancy[10][11].bits.building = df::tile_building_occ::Planned; break;
        }
        ineligible();
    }
}
void item_reference_validation() {
    df::general_ref general; df::specific_ref specific; df::job job; job.id = 1;
    item().general_refs.push_back(&general); ineligible();
    reset(); item().specific_refs.push_back(&specific); ineligible();
    reset(); item().general_refs.push_back(nullptr); denied(read(), 5);
    reset(); item().specific_refs.push_back(nullptr); denied(read(), 5);
    reset(); specific.type = df::specific_ref_type::JOB; item().specific_refs.push_back(&specific); denied(read(), 5);
    reset(); specific.data.job = &job; item().specific_refs = {&specific, &specific}; denied(read(), 5);
    reset(); item().general_refs.assign(MAX_REFS + 1, &general); denied(read(), 5);
    reset(); item().specific_refs.assign(9, &specific); denied(read(), 5);
    CHECK(mock::writer_calls == 0);
}
void building_registry_integrity() {
    for (int fault = 0; fault < 7; ++fault) {
        reset(); auto *b = existing();
        switch (fault) {
            case 0: world.buildings.all.push_back(nullptr); break;
            case 1: world.buildings.all.push_back(b); break; case 2: b->id = -1; break;
            case 3: b->id = mock::next_building; break; case 4: b->x2 = b->x1 - 1; break;
            case 5: b->z = 32768; break; case 6: world.buildings.all.resize(bp::MAX_BUILDINGS + 1, b); break;
        }
        denied(read(), 5); CHECK(mock::writer_calls == 0);
    }
}
void building_zone_and_stockpile_overlap() {
    for (auto type : {df::building_type::Bed, df::building_type::Stockpile, df::building_type::Civzone}) {
        reset(); auto *b = existing(1, 15, 15, type); b->x1 = b->y1 = 13; b->x2 = b->y2 = 17; ineligible();
    }
    reset(); existing(1, 15, 15); existing(2, 15, 15); denied(read(), 4);
    reset(); auto *outside = existing(); const auto q = prepared(); CHECK(commit(q).accepted);
    CHECK(world.buildings.all.front() == outside && outside->id == 1 && outside->jobs.empty());
}
void zone_extent_and_registry_validation() {
    auto create_zone = [] {
        auto *zone = new df::building_civzonest(); zone->id = 1;
        zone->x1 = zone->x2 = 25; zone->y1 = zone->y2 = 25; zone->z = 2;
        zone->room = {14, 14, 3, 3}; world.buildings.all.push_back(zone);
        world.buildings.other.ANY_ZONE.push_back(zone); return zone;
    };
    create_zone(); ineligible();
    for (int fault = 0; fault < 8; ++fault) {
        reset(); auto *zone = create_zone();
        switch (fault) {
            case 0: world.buildings.other.ANY_ZONE.push_back(nullptr); break;
            case 1: world.buildings.all.clear(); break;
            case 2: world.buildings.other.ANY_ZONE.push_back(zone); break;
            case 3: zone->type = df::building_type::Bed; break;
            case 4: zone->room.width = -1; break; case 5: zone->room.x = -1; break;
            case 6: zone->room.width = INT32_MAX; break;
            case 7: world.buildings.other.ANY_ZONE.resize(bp::MAX_BUILDINGS + 1, zone); break;
        }
        denied(read(), 5); CHECK(mock::writer_calls == 0);
    }
    reset(); auto *zone = create_zone(); zone->room = {25, 25, 1, 1};
    const auto q = prepared(); CHECK(commit(q).accepted && retained(q).phase == bp::Phase::Placed);
    CHECK(zone->relations.empty());
}
void computed_item_flags_remain_witnessed() {
    for (unsigned flags : {0u, 1u << 28, 1u << 29, (1u << 28) | (1u << 29)}) {
        reset(); item().flags.whole |= flags;
        const auto q = prepared(); CHECK(retained(q).before.item.other_flags == flags);
        CHECK(commit(q).accepted && retained(q).phase == bp::Phase::Placed);
        CHECK(retained(q).after->item.other_flags == flags && mock::writer_calls == 1);
    }
    reset(); const auto q = prepared(); item().flags.whole |= 1u << 28;
    CHECK(commit(q).accepted && retained(q).phase == bp::Phase::Refused && mock::writer_calls == 0);
}
void successful_furniture(unsigned kind) {
    wire::Request request; request.kk = kind; item().type = static_cast<df::item_type>(kind);
    auto other = std::make_unique<df::item>(); other->id = 43; other->pos = {22, 23, 2};
    auto *unselected = other.get(); world.items.all.push_back(unselected); mock::items.push_back(std::move(other));
    const auto q = prepared(request); CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
    const auto before = retained(q).before; const auto reply = commit(q); CHECK(reply.accepted && !reply.unresolved);
    CHECK(retained(q).phase == bp::Phase::Placed && retained(q).reason == bp::Reason::None);
    CHECK(retained(q).after && retained(q).insertion);
    CHECK(retained(q).after->encode() == before.expected_after().encode());
    CHECK(retained(q).insertion->matches(before)); CHECK(mock::writer_calls == 1 && mock::allocator_calls == 1);
    CHECK(world.buildings.all.size() == 1 && mock::jobs.size() == 1);
    const auto *b = world.buildings.all.front(); const auto *j = mock::jobs.front().get();
    CHECK(b->id == 70 && b->getType() == static_cast<df::building_type>(kind));
    CHECK(b->x1 == 15 && b->x2 == 15 && b->y1 == 15 && b->y2 == 15 && b->z == 2);
    CHECK(b->mat_type == 419 && b->mat_index == -1 && b->getBuildStage() == 0 && b->getMaxBuildStage() == 1);
    CHECK(b->jobs.size() == 1 && b->jobs.front() == j && j->id == 90 && j->job_type == df::job_type::ConstructBuilding);
    CHECK(j->list_link && j->list_link->item == j && world.jobs.list.next == j->list_link);
    CHECK(j->items.size() == 1 && j->items.front()->item == &item());
    CHECK(j->items.front()->role == df::job_role_type::Hauled && j->items.front()->job_item_idx == -1);
    CHECK(item().specific_refs.size() == 1 && item().specific_refs.front()->data.job == j && item().flags.bits.in_job);
    CHECK(!unselected->flags.bits.in_job && unselected->specific_refs.empty() && unselected->pos == df::coord(22, 23, 2));
    CHECK(block().occupancy[15][15].bits.building == df::tile_building_occ::Planned && block().tiletype[15][15] == df::tiletype::Floor);
    CHECK(mock::next_building == 71 && mock::next_job == 91 && mock::free_arguments_safe);
    CHECK(reply.records == 1 && reply.effect == retained(q).encode());
}
void stale_world_capture() {
    for (int fault = 0; fault < 10; ++fault) {
        reset(); const auto q = prepared();
        switch (fault) {
            case 0: ++mock::tick; break; case 1: ++mock::year; break; case 2: ++mock::site; break;
            case 3: mock::folder = "another-world"; break; case 4: mock::paused = false; break;
            case 5: ++mock::next_building; break; case 6: ++mock::next_job; break;
            case 7: existing(); break; case 8: --mock::size_x; break; case 9: mock::loaded = false; break;
        }
        const auto reply = commit(q); CHECK(reply.accepted && !reply.unresolved);
        CHECK(retained(q).phase == bp::Phase::Refused && retained(q).reason == bp::Reason::Stale);
        CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
    }
}
void stale_item_capture() {
    for (int fault = 0; fault < 8; ++fault) {
        reset(); const auto q = prepared();
        switch (fault) {
            case 0: ++item().quality; break; case 1: ++item().material; break; case 2: ++item().material_index; break;
            case 3: ++item().subtype; break; case 4: ++item().pos.x; break; case 5: item().flags.bits.forbid = true; break;
            case 6: item().flags2.whole = 1; break; case 7: block(10, 11).occupancy[10][11].bits.other = 1; break;
        }
        CHECK(commit(q).accepted); CHECK(retained(q).phase == bp::Phase::Refused);
        CHECK(retained(q).reason == bp::Reason::Stale && mock::writer_calls == 0 && mock::allocator_calls == 0);
    }
}
void stale_terrain_capture() {
    for (int fault = 0; fault < 6; ++fault) {
        reset(); const auto q = prepared();
        switch (fault) {
            case 0: block(14, 14).tiletype[14][14] = df::tiletype::Wall; break;
            case 1: block(14, 14).occupancy[14][14].bits.item = 1; break;
            case 2: block(14, 14).designation[14][14].bits.dig = df::tile_dig_designation::Default; break;
            case 3: mock::supported = false; break; case 4: mock::free_tile = false; break;
            case 5: block(14, 14).designation[14][14].bits.hidden = true; break;
        }
        CHECK(commit(q).accepted); CHECK(retained(q).phase == bp::Phase::Refused && retained(q).reason == bp::Reason::Stale);
        CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
    }
}
void immediate_prewrite_revalidation() {
    const auto q = prepared(); mock::after_allocation = [] { ++item().quality; };
    indeterminate(q, commit(q)); CHECK(mock::allocator_calls == 1 && mock::writer_calls == 0);
    CHECK(world.buildings.all.empty() && df::live_buildings.empty());
}
void retained_replay_without_game_access() {
    const auto q = prepared(); const auto preparation = retained(q).encode(); const auto created = retained(q).created_ms;
    mock::game_access = 0; const auto replay = call(PreparePlacement, q);
    CHECK(replay.accepted && replay.replayed && replay.effect == preparation);
    CHECK(retained(q).created_ms == created && mock::game_access == 0);
    CHECK(commit(q).accepted); const auto receipt = retained(q).encode();
    mock::game_access = 0; mock::loaded = false;
    CHECK(commit(q).effect == receipt); CHECK(query(q).effect == receipt); CHECK(cancel(q).effect == receipt);
    CHECK(mock::game_access == 0 && mock::writer_calls == 1 && engine.size() == 1);
    auto changed = q; changed.plan[0] ^= 1; denied(query(changed), 7); CHECK(mock::game_access == 0);
}
void cancellation_and_token_authentication() {
    auto q = prepared(); auto bad = q; bad.token[0] ^= 1;
    denied(commit(bad), 7); denied(cancel(bad), 7); CHECK(retained(q).phase == bp::Phase::Prepared);
    mock::game_access = 0; const auto reply = cancel(q); CHECK(reply.accepted && !reply.unresolved);
    CHECK(retained(q).phase == bp::Phase::Cancelled && retained(q).reason == bp::Reason::Cancelled);
    CHECK(commit(q).effect == reply.effect && cancel(q).effect == reply.effect && query(q).effect == reply.effect);
    CHECK(mock::game_access == 0 && mock::writer_calls == 0);
    q.key = "missing"; denied(commit(q), 7); const auto absent = query(q);
    CHECK(absent.accepted && absent.effect.empty() && absent.records == 1);
}
void expiration_without_sleep() {
    const auto q = prepared();
    // Expose a deterministic elapsed lifetime through the already-public engine
    // input, without a fake clock in production or a wall-clock sleep.
    auto *record = const_cast<bp::Record *>(engine.query(q.key, q.plan)); CHECK(record);
    record->created_ms = monotonic_ms() - bp::PREPARE_MS;
    const auto reply = commit(q); CHECK(reply.accepted);
    CHECK(retained(q).phase == bp::Phase::Refused && retained(q).reason == bp::Reason::Expired);
    CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
}
void source_changes_preserve_records() {
    for (auto event : {SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED}) {
        reset(); const auto q = prepared(); const auto generation = engine.generation();
        CHECK(plugin_onstatechange(output, event) == CR_OK); CHECK(engine.generation() == generation + 1);
        CHECK(query(q).accepted && engine.size() == 1); CHECK(commit(q).accepted);
        CHECK(retained(q).phase == bp::Phase::Refused && retained(q).reason == bp::Reason::SourceChanged);
        CHECK(mock::writer_calls == 0);
    }
    reset(); const auto q = prepared(); CHECK(plugin_shutdown(output) == CR_OK);
    CHECK(engine.size() == 1 && commit(q).accepted && retained(q).reason == bp::Reason::SourceChanged);
}
void pause_events_invalidate_preparations() {
    for (auto event : {SC_PAUSED, SC_UNPAUSED}) {
        reset(); const auto q = prepared(); const auto generation = engine.generation();
        CHECK(plugin_onstatechange(output, event) == CR_OK); CHECK(engine.generation() == generation);
        CHECK(commit(q).accepted && retained(q).phase == bp::Phase::Refused);
        CHECK(retained(q).reason == bp::Reason::Stale && mock::writer_calls == 0);
    }
    reset(); const auto q = prepared(); CHECK(plugin_onstatechange(output, SC_OTHER) == CR_OK);
    CHECK(commit(q).accepted && retained(q).phase == bp::Phase::Placed);
}
void partial_write_uncertainty_fence() {
    for (auto fault : {mock::WriteFault::BuildingOnly, mock::WriteFault::JobOnly}) {
        reset(); const auto first = prepared(); wire::Request second; second.key = "second"; second.xx = 20; second.yy = 20;
        const auto pending = prepared(second); mock::write_fault = fault;
        indeterminate(first, commit(first)); CHECK(mock::writer_calls == 1);
        CHECK(world.buildings.all.size() == 1); CHECK(mock::next_building == 71);
        const auto historical = retained(first).encode(); CHECK(commit(first).effect == historical);
        CHECK(cancel(first).effect == historical && query(first).effect == historical);
        denied(commit(pending), 8); CHECK(mock::writer_calls == 1);
        CHECK(plugin_onstatechange(output, SC_WORLD_UNLOADED) == CR_OK);
        CHECK(query(first).effect == historical && engine.unresolved() && engine.size() == 2);
        second.key = "third"; const auto fresh = candidate(second); denied(call(PreparePlacement, fresh), 8);
        CHECK(cancel(pending).accepted && engine.unresolved());
    }
}
void no_application_is_still_indeterminate() {
    for (auto fault : {mock::WriteFault::FalseBefore, mock::WriteFault::ThrowBefore}) {
        reset(); const auto q = prepared(); mock::write_fault = fault; indeterminate(q, commit(q));
        CHECK(world.buildings.all.empty() && mock::jobs.empty() && !item().flags.bits.in_job);
        CHECK(mock::writer_calls == 1); const auto old = retained(q).encode();
        wire::Request another; another.key = "other"; const auto next = candidate(another);
        denied(call(PreparePlacement, next), 8); CHECK(commit(q).effect == old && mock::writer_calls == 1);
    }
}
void completed_native_call_requires_observed_proof() {
    for (auto fault : {mock::WriteFault::FalseAfter, mock::WriteFault::ThrowAfter}) {
        reset(); const auto q = prepared(); mock::write_fault = fault;
        const auto r = commit(q); CHECK(r.accepted && !r.unresolved); CHECK(retained(q).phase == bp::Phase::Placed);
        CHECK(mock::writer_calls == 1 && retained(q).insertion && retained(q).after);
    }
}
void building_insertion_verification() {
    for (int fault = 0; fault < 12; ++fault) {
        reset(); const auto q = prepared();
        mock::after_construct = [fault](df::building *b, df::job *, df::item *) {
            switch (fault) {
                case 0: b->type = df::building_type::Chair; break; case 1: ++b->centerx; break;
                case 2: ++b->centery; break; case 3: ++b->x2; break; case 4: ++b->mat_type; break;
                case 5: ++b->mat_index; break; case 6: b->stage = 1; break;
                case 7: b->maximum_stage = 0; break; case 8: b->maximum_stage = 33; break;
                case 9: b->jobs.push_back(b->jobs.front()); break;
                case 10: b->general_refs.push_back(nullptr); break; case 11: b->relations.push_back(nullptr); break;
            }
        };
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void job_insertion_verification() {
    for (int fault = 0; fault < 13; ++fault) {
        reset(); const auto q = prepared();
        mock::after_construct = [fault](df::building *, df::job *j, df::item *) {
            switch (fault) {
                case 0: j->job_type = df::job_type::DestroyBuilding; break; case 1: ++j->pos.x; break;
                case 2: ++j->mat_type; break; case 3: ++j->mat_index; break;
                case 4: j->flags.bits.suspend = true; break; case 5: j->flags.bits.repeat = true; break;
                case 6: j->job_items.elements.push_back(nullptr); break; case 7: j->general_refs.front()->type = df::general_ref_type::NONE; break;
                case 8: j->general_refs.front()->holder = nullptr; break; case 9: j->general_refs.push_back(nullptr); break;
                case 10: j->list_link = nullptr; break; case 11: j->specific_refs.push_back(nullptr); break;
                case 12: j->items.push_back(j->items.front()); break;
            }
        };
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void item_attachment_verification() {
    for (int fault = 0; fault < 8; ++fault) {
        reset(); const auto q = prepared();
        mock::after_construct = [fault](df::building *, df::job *j, df::item *i) {
            switch (fault) {
                case 0: j->items.front()->item = nullptr; break;
                case 1: j->items.front()->role = df::job_role_type::Reagent; break;
                case 2: j->items.front()->job_item_idx = 0; break; case 3: j->items.front()->flags.whole = 1; break;
                case 4: i->specific_refs.clear(); break; case 5: i->specific_refs.front()->data.job = nullptr; break;
                case 6: i->specific_refs.front()->type = df::specific_ref_type::UNIT; break;
                case 7: i->flags.bits.in_job = false; break;
            }
        };
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void complete_job_registry_verification() {
    for (int fault = 0; fault < 4; ++fault) {
        reset(); const auto q = prepared();
        mock::after_construct = [fault](df::building *, df::job *j, df::item *) {
            if (fault == 0) world.jobs.list.next = nullptr;
            if (fault == 1) j->list_link->next = j->list_link;
            if (fault >= 2) {
                auto extra = std::make_unique<df::job_list_link>();
                extra->item = fault == 2 ? nullptr : j; j->list_link->next = extra.get();
                mock::links.push_back(std::move(extra));
            }
        };
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void planned_occupancy_verification() {
    for (unsigned occupancy : {0u, 2u, 3u, 7u}) {
        reset(); const auto q = prepared();
        mock::after_construct = [occupancy](df::building *, df::job *, df::item *) {
            block().occupancy[15][15].bits.building = static_cast<df::tile_building_occ>(occupancy);
        };
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void final_readback_after_native_verification() {
    const auto q = prepared(); mock::during_verify = [](df::job *) { block(14, 14).tiletype[14][14] = df::tiletype::Wall; };
    indeterminate(q, commit(q)); CHECK(mock::holder_calls == 1 && mock::writer_calls == 1);
}
void lost_reply_is_queryable() {
    const auto q = prepared(); mock::reply_failure = true; denied(commit(q), 5);
    CHECK(retained(q).phase == bp::Phase::Placed && mock::writer_calls == 1 && !engine.unresolved());
    const auto recovered = query(q); CHECK(recovered.accepted && recovered.effect == retained(q).encode());
    CHECK(commit(q).effect == recovered.effect && mock::writer_calls == 1);
}
void retained_record_limit_without_eviction() {
    const auto first = prepared(); CHECK(cancel(first).accepted);
    auto next = candidate();
    for (std::size_t i = 1; i < bp::MAX_RECORDS; ++i) {
        next.key = "record-" + std::to_string(i);
        const auto reply = call(PreparePlacement, next); CHECK(reply.accepted && reply.records == i + 1);
        next.token = retained(next).token; CHECK(cancel(next).accepted);
    }
    CHECK(engine.size() == bp::MAX_RECORDS && mock::writer_calls == 0 && mock::allocator_calls == 0);
    next.key = "overflow"; denied(call(PreparePlacement, next), 5);
    CHECK(query(first).effect == retained(first).encode() && engine.size() == bp::MAX_RECORDS);
    const auto status = call(Handshake, {}); CHECK(status.accepted && status.records == bp::MAX_RECORDS && !status.unresolved);
}
void native_allocation_failure_retains_uncertainty() {
    for (int fault = 0; fault < 2; ++fault) {
        reset(); const auto q = prepared();
        if (fault == 0) mock::allocation_failure = true; else mock::resize_failure = true;
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 0 && mock::allocator_calls == 1);
        CHECK(world.buildings.all.empty() && df::live_buildings.empty());
    }
}
void handshake_exposes_retained_uncertainty() {
    const auto q = prepared(); mock::write_fault = mock::WriteFault::BuildingOnly; indeterminate(q, commit(q));
    unsetenv("DFMCP_BUILD_ALLOW_PLACE"); mock::game_access = 0;
    const auto status = call(Handshake, {}); CHECK(status.accepted && status.unresolved && status.records == 1);
    CHECK(query(q).accepted && cancel(q).accepted && mock::game_access == 0 && mock::writer_calls == 1);
    CHECK(plugin_shutdown(output) == CR_OK); CHECK(engine.unresolved());
    CHECK(call(Handshake, {}).unresolved && query(q).effect == retained(q).encode());
}
std::string hex(const std::string &value) {
    constexpr char digits[] = "0123456789abcdef";
    std::string result; result.reserve(value.size() * 2);
    for (unsigned char byte : value) { result.push_back(digits[byte >> 4]); result.push_back(digits[byte & 15]); }
    return result;
}
void emit_vectors() {
    reset(); const auto observation = read(); const auto q = prepared();
    const auto preparation = query(q); const auto placed = commit(q);
    CHECK(observation.accepted && preparation.accepted && placed.accepted);
    CHECK(retained(q).phase == bp::Phase::Placed);
    std::cout << "{\"capture\":\"" << hex(observation.observation)
        << "\",\"prepared\":\"" << hex(preparation.effect) << "\",\"placed\":\"" << hex(placed.effect)
        << "\",\"plan\":\"" << hex(q.plan) << "\",\"token\":\"" << hex(q.token) << "\"}\n";
    cleanup();
}
}

int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--vectors") { emit_vectors(); return 0; }
        CHECK(argc == 1);
        run("rpc_manifest_and_suspended_dispatch", rpc_manifest);
        run("strict_development_environment_gate", environment_gates);
        run("production_admission_marker_cannot_authorize_development", admitted_process_isolation);
        run("bounded_exact_bearer_authentication", bearer_bounds_and_equality);
        run("exact_protocol_and_nonce_bounds", protocol_and_nonce);
        run("message_initialization_unknown_fields_and_frame_bound", message_initialization_and_unknown_fields);
        run("exhaustive_handshake_and_read_shapes", read_shapes);
        run("exhaustive_mutation_query_and_cancel_shapes", effect_shapes);
        run("placement_authority_and_gate_disabled_recovery", placement_authority_and_recovery);
        run("bounded_utf8_native_manifest", manifest_validation);
        run("bounded_reply_and_failure_clearing", bounded_reply_and_clearing);
        run("unavailable_fortress_refusal", unavailable_world);
        run("world_identity_and_horizon_validation", malformed_world_identity);
        run("selection_coordinate_and_dimension_bounds", bounded_selection);
        run("hidden_terrain_attribute_redaction", hidden_terrain_redaction);
        run("missing_terrain_remains_unknown", missing_terrain_refusal);
        run("hidden_item_attribute_redaction", hidden_item_redaction);
        run("unavailable_and_invalid_exact_item", unavailable_item_selection);
        run("paused_supported_free_floor_policy", target_floor_policy);
        run("target_occupancy_and_designation_policy", target_occupancy_and_designation_policy);
        run("dry_visible_context_and_adjacent_floor", neighborhood_eligibility);
        run("unclaimed_unworn_ground_item_policy", item_availability_policy);
        run("bounded_complete_item_reference_validation", item_reference_validation);
        run("complete_bounded_building_registry", building_registry_integrity);
        run("building_zone_and_stockpile_overlap", building_zone_and_stockpile_overlap);
        run("zone_room_extents_and_complete_native_registry", zone_extent_and_registry_validation);
        run("computed_item_flags_preserved_and_revalidated", computed_item_flags_remain_witnessed);
        run("exact_bed_registry_job_and_item_insertion", [] { successful_furniture(1); });
        run("exact_chair_registry_job_and_item_insertion", [] { successful_furniture(2); });
        run("exact_table_registry_job_and_item_insertion", [] { successful_furniture(3); });
        run("complete_world_revalidation_before_write", stale_world_capture);
        run("complete_selected_item_revalidation_before_write", stale_item_capture);
        run("complete_terrain_revalidation_before_write", stale_terrain_capture);
        run("revalidation_after_allocation_before_native_writer", immediate_prewrite_revalidation);
        run("retained_replay_never_accesses_game", retained_replay_without_game_access);
        run("authenticated_cancellation_never_writes", cancellation_and_token_authentication);
        run("expired_preparation_never_writes", expiration_without_sleep);
        run("source_changes_preserve_records_and_refuse_old_plan", source_changes_preserve_records);
        run("pause_sequence_fences_preparation", pause_events_invalidate_preparations);
        run("partial_write_fenced_across_keys_and_source_changes", partial_write_uncertainty_fence);
        run("native_nonapplication_cannot_prove_no_effect", no_application_is_still_indeterminate);
        run("independent_readback_handles_false_and_throwing_native_returns", completed_native_call_requires_observed_proof);
        run("exact_native_building_proof", building_insertion_verification);
        run("exact_native_job_and_holder_proof", job_insertion_verification);
        run("exact_hauled_item_and_reverse_link_proof", item_attachment_verification);
        run("complete_job_registry_and_link_proof", complete_job_registry_verification);
        run("exact_planned_building_occupancy_proof", planned_occupancy_verification);
        run("final_capture_after_insertion_verification", final_readback_after_native_verification);
        run("lost_commit_response_is_queryable_without_replay", lost_reply_is_queryable);
        run("bounded_lifetime_record_retention_without_eviction", retained_record_limit_without_eviction);
        run("allocation_failures_are_retained_without_native_retry", native_allocation_failure_retains_uncertainty);
        run("handshake_reports_unresolved_retained_work", handshake_exposes_retained_uncertainty);
        cleanup();
        std::cout << "{\"groups\":" << groups.size() << ",\"assertions\":" << checks << ",\"group_names\":[";
        for (std::size_t i = 0; i < groups.size(); ++i) std::cout << (i ? "," : "") << '"' << groups[i] << '"';
        std::cout << "]}\n"; return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n'; cleanup(); return 1;
    }
}
