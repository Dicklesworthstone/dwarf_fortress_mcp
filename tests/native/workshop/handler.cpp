#include "stubs.h"
#include "../../../bridge/dfhack-workshop-v1_20/dfmcp_workshop_v1_20.cpp"
#include <cstring>
#include <iomanip>
#include <iostream>
#include <sstream>

namespace {
std::size_t checks = 0;
std::vector<std::string> groups;
std::string current_group;
#define CHECK(condition) do { ++checks; if (!(condition)) throw std::runtime_error( \
    "check failed: " + current_group + ":" + std::to_string(__LINE__) + ": " #condition); } while (false)
color_ostream output;
df::world world;
using Rpc = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
const std::string TOKEN(32, 's');

void cleanup() {
    world.buildings.other.ANY_ZONE.clear(); world.buildings.all.clear(); world.items.all.clear();
    world.units.active.clear(); world.jobs.list.next = nullptr;
    while (!df::live_buildings.empty()) delete *df::live_buildings.begin();
    mock::jobs.clear(); mock::links.clear(); mock::general_refs.clear(); mock::contained.clear();
    mock::specific_refs.clear(); mock::item_refs.clear(); mock::items.clear(); mock::units.clear();
    mock::blocks.clear(); mock::walk_groups.clear(); mock::materials.clear();
}
df::map_block &block(int x, int y, int z = 2) { return *mock::blocks.at({x / 16, y / 16, z}); }
df::tile_designation &designation(int x, int y) { return block(x, y).designation[x & 15][y & 15]; }
df::tile_occupancy &occupancy(int x, int y) { return block(x, y).occupancy[x & 15][y & 15]; }
df::unit &add_unit(std::int32_t id, int x, int y, bool labor = true) {
    mock::units.push_back(std::make_unique<df::unit>()); auto &u = *mock::units.back();
    u.id = id; u.pos = {x, y, 2}; u.status.labors[df::unit_labor::CARPENTER] = labor;
    world.units.active.push_back(&u); occupancy(x, y).bits.unit = 1;
    mock::walk_groups[{x, y, 2}] = 1; return u;
}
void reset() {
    cleanup(); engine = ws::Engine(7);
    mock::loaded = mock::mapped = mock::fortress = mock::paused = mock::initialized = true;
    mock::unknown = mock::reply_failure = mock::oversized_reply = mock::oversized_request = false;
    mock::free_tiles = mock::supported = mock::helper_arguments_safe = true;
    mock::setting_occupancy = mock::needs_design = true;
    mock::allocation_failure = mock::resize_failure = mock::invalid_dfhack_version = false;
    mock::year = 0; mock::tick = 12345; mock::site = 1;
    mock::size_x = mock::size_y = 64; mock::size_z = 8;
    mock::folder = "region1"; mock::df_version = "test-df"; mock::dfhack_version = "test-dfhack";
    mock::next_building = 70; mock::next_job = 90;
    mock::write_fault = mock::WriteFault::None;
    mock::after_construct = {}; mock::after_allocation = {}; mock::during_verify = {};
    df::global::world = &world; df::global::plotinfo = &world;
    df::global::building_next_id = &mock::next_building; df::global::job_next_id = &mock::next_job;
    Core::getInstance().vinfo = std::make_shared<VersionInfo>();
    for (int y = 0; y < 2; ++y) for (int x = 0; x < 2; ++x)
        mock::blocks[{x, y, 2}] = std::make_unique<df::map_block>();
    mock::materials[{420, 163}].flags.values = {df::material_flags::WOOD};
    mock::items.push_back(std::make_unique<df::item>()); world.items.all.push_back(mock::items.back().get());
    occupancy(10, 11).bits.item = 1;
    mock::walk_groups[{15, 15, 2}] = 1; mock::walk_groups[{10, 11, 2}] = 1;
    add_unit(3, 20, 20);
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    setenv("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20", "1", 1);
    setenv("DFMCP_WORKSHOP_ALLOW_BUILD", "1", 1);
    setenv("DFMCP_WORKSHOP_TOKEN", TOKEN.c_str(), 1);
    mock::game_access = mock::writer_calls = mock::allocator_calls = mock::walk_calls = mock::helper_calls = 0;
}
df::item &item() { return *mock::items.front(); }
wire::Reply call(Rpc method, const wire::Request &request) {
    wire::Reply reply; CHECK(method(output, &request, &reply) == CR_OK); return reply;
}
void denied(const wire::Reply &reply, unsigned code) {
    CHECK(!reply.accepted); CHECK(reply.code == code); CHECK(reply.mask == 0);
    CHECK(reply.effect.empty() && reply.observation.empty());
    CHECK(reply.generation == 0 && reply.df_version.empty() && reply.dfhack_version.empty());
    CHECK(reply.major == 1 && reply.minor == 20 && reply.nonce.size() <= 64);
}
wire::Reply read(wire::Request request = {}) { request.mask = READ; return call(ReadWorkshop, request); }
ws::Capture observed(wire::Request request = {}) {
    const auto reply = read(request); CHECK(reply.accepted);
    return engine.inspect(selection(&request), capture);
}
wire::Request candidate(wire::Request request = {}) {
    request.mask = READ;
    const auto reply = read(request); CHECK(reply.accepted); CHECK(!reply.observation.empty());
    request.witness = dfmcp_snapshot::sha256(reply.observation);
    request.plan = ws::plan_digest(selection(&request), request.witness); request.mask = PREPARE;
    return request;
}
wire::Request prepared(wire::Request request = {}) {
    request = candidate(request); const auto reply = call(PrepareWorkshop, request);
    CHECK(reply.accepted); CHECK(!reply.replayed && !reply.unresolved); CHECK(reply.records == engine.size());
    const auto *record = engine.query(request.key, request.plan); CHECK(record);
    CHECK(record->phase == ws::Phase::Prepared); request.token = record->token;
    return request;
}
wire::Reply commit(wire::Request request) { request.mask = COMMIT; return call(CommitWorkshop, request); }
wire::Reply query(wire::Request request) { request.mask = QUERY; return call(QueryWorkshop, request); }
wire::Reply cancel(wire::Request request) { request.mask = COMMIT; return call(CancelWorkshop, request); }
wire::Reply progress_reply(std::uint32_t building = 70, std::uint32_t item_id = 42) {
    wire::Request request; request.mask = PROGRESS; request.building = building; request.item = item_id;
    return call(ReadWorkshopProgress, request);
}
const ws::Record &retained(const wire::Request &request) {
    const auto *record = engine.query(request.key, request.plan); CHECK(record); return *record;
}
void ineligible() {
    const auto q = candidate(); denied(call(PrepareWorkshop, q), 4);
    CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0 && engine.size() == 0);
}
df::building *existing(int id, int x, int y) {
    auto *b = new df::building_actual(); b->id = id; b->type = df::building_type::Bed;
    b->x1 = b->x2 = b->centerx = x; b->y1 = b->y2 = b->centery = y; b->z = 2;
    world.buildings.all.push_back(b); return b;
}
void indeterminate(const wire::Request &q, const wire::Reply &r) {
    CHECK(r.accepted && r.unresolved); CHECK(retained(q).phase == ws::Phase::Indeterminate);
    CHECK(retained(q).reason == ws::Reason::NativeFailure); CHECK(!retained(q).after && !retained(q).insertion);
    CHECK(engine.unresolved()); CHECK(r.effect == retained(q).encode());
}
df::building_workshopst *placed_workshop() {
    for (auto *b : world.buildings.all) if (b->id == 70) return dynamic_cast<df::building_workshopst *>(b);
    return nullptr;
}
void run(const char *name, const std::function<void()> &body) {
    current_group = name; reset(); body(); groups.emplace_back(name);
}

void rpc_manifest() {
    std::vector<PluginCommand> commands; CHECK(plugin_init(output, commands) == CR_OK); CHECK(commands.empty());
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));
    const std::array<std::string, 7> names{"Handshake", "ReadWorkshop", "PrepareWorkshop",
        "CommitWorkshop", "QueryWorkshop", "CancelWorkshop", "ReadWorkshopProgress"};
    CHECK(service->methods.size() == names.size());
    for (std::size_t i = 0; i < names.size(); ++i) {
        CHECK(service->methods[i].first == names[i]); CHECK(service->methods[i].second == 0);
    }
    const auto reply = call(Handshake, {});
    CHECK(reply.accepted && reply.code == 0 && reply.generation == 7 && reply.minor == 20);
    CHECK(reply.df_version == "test-df" && reply.dfhack_version == "test-dfhack");
    CHECK(reply.mask == 24 && !reply.unresolved && reply.records == 0); CHECK(mock::game_access == 0);
}
void environment_and_profile_gates() {
    for (const char *value : {"", "0", "true", "01", "1 "}) {
        setenv("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20", value, 1);
        denied(call(Handshake, {}), 1); CHECK(mock::game_access == 0);
    }
    unsetenv("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20"); denied(call(Handshake, {}), 1);
    reset();
    // The furniture/1.19 switch never opens this generation.
    unsetenv("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20"); setenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19", "1", 1);
    denied(call(Handshake, {}), 1); unsetenv("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19");
    reset();
    for (const char *value : {"", "1.0", "1.20"}) {
        setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", value, 1);
        denied(call(Handshake, {}), 1); CHECK(mock::game_access == 0);
    }
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    for (unsigned minor : {19u, 21u, 0u}) { wire::Request q; q.minor = minor; denied(call(Handshake, q), 2); }
    { wire::Request q; q.major = 2; denied(call(Handshake, q), 2); }
    for (unsigned length : {0u, 31u, 257u}) { wire::Request q; q.bearer.assign(length, 's'); denied(call(Handshake, q), 1); }
    { wire::Request q; q.bearer[31] = 'x'; denied(call(Handshake, q), 1); }
    for (unsigned length : {0u, 15u, 65u}) { wire::Request q; q.nonce.assign(length, 'n'); denied(call(Handshake, q), 3); }
    mock::unknown = true; denied(call(Handshake, {}), 3); mock::unknown = false;
    mock::initialized = false; denied(call(Handshake, {}), 3); mock::initialized = true;
    mock::oversized_request = true; denied(call(Handshake, {}), 3); mock::oversized_request = false;
    // Build permission gates prepare/commit only; reads, query and cancel stay available.
    const auto q = prepared(); unsetenv("DFMCP_WORKSHOP_ALLOW_BUILD");
    denied(commit(q), 1); CHECK(read().accepted); CHECK(query(q).accepted);
    CHECK(progress_reply().accepted); CHECK(cancel(q).accepted && retained(q).phase == ws::Phase::Cancelled);
    CHECK(mock::writer_calls == 0 && mock::allocator_calls == 0);
}
void exact_request_shapes() {
    const std::array<std::pair<Rpc, unsigned>, 7> methods{{{Handshake, 0u}, {ReadWorkshop, READ},
        {PrepareWorkshop, PREPARE}, {CommitWorkshop, COMMIT}, {QueryWorkshop, QUERY},
        {CancelWorkshop, COMMIT}, {ReadWorkshopProgress, PROGRESS}}};
    for (const auto &[method, expected] : methods)
        for (unsigned mask = 0; mask < 512; ++mask) {
            if (mask == expected) continue;
            wire::Request q; q.mask = mask; denied(call(method, q), 3);
        }
    CHECK(mock::game_access == 0 && engine.size() == 0);
}
void positive_construction_and_completion() {
    const auto before = observed();
    CHECK(before.eligible() && before.census_complete && before.workers.size() == 1);
    CHECK(before.workers[0].id == 3 && before.workers[0].labor && before.workers[0].reachable);
    CHECK(before.item.log && before.item.wood && before.item.reachable && before.item.available());
    CHECK(before.free_footprint && before.supported && mock::helper_arguments_safe);
    for (const auto &t : before.tiles) CHECK(t.empty_floor());
    const auto q = prepared(); const auto writes = mock::writer_calls;
    CHECK(writes == 0);
    const auto reply = commit(q);
    CHECK(reply.accepted && !reply.unresolved && mock::writer_calls == 1 && mock::allocator_calls == 1);
    const auto &record = retained(q);
    CHECK(record.phase == ws::Phase::Placed && record.after && record.insertion);
    CHECK(record.after->encode() == record.before.expected_after().encode());
    const auto &proof = *record.insertion;
    CHECK(proof.building == 70 && proof.job == 90 && proof.item == 42 && proof.stage == 0 && proof.max_stage == 3);
    CHECK(proof.x1 == 14 && proof.x2 == 16 && proof.y1 == 14 && proof.y2 == 16 && proof.center_x == 15 && proof.center_y == 15);
    CHECK(proof.material == 420 && proof.material_index == 163 && proof.planned_tiles == ws::ALL_TILES);
    CHECK(proof.setting_occupancy && proof.has_design && proof.building_flags == 0 && !proof.suspended);
    for (int x = 14; x <= 16; ++x) for (int y = 14; y <= 16; ++y)
        CHECK(occupancy(x, y).bits.building == df::tile_building_occ::Planned);
    auto *workshop = placed_workshop();
    CHECK(workshop && workshop->type == df::workshop_type::Carpenters && workshop->custom_type == -1);
    CHECK(workshop->jobs.size() == 1 && workshop->jobs[0]->items[0]->item == &item() && item().flags.bits.in_job);
    CHECK(reply.effect == record.encode());
    // Duplicate commit and query return immutable history with no second writer.
    CHECK(commit(q).effect == reply.effect && query(q).effect == reply.effect && mock::writer_calls == 1);
    CHECK(cancel(q).effect == reply.effect && retained(q).phase == ws::Phase::Placed);
    // Stage zero is construction registration, not completion.
    const auto first = progress_reply(); CHECK(first.accepted && first.mask == 25);
    const auto staged = engine.progress(70, 42, progress);
    CHECK(staged.present && staged.carpenters && staged.stage == 0 && staged.max_stage == 3);
    CHECK(staged.construct_jobs == 1 && !staged.exists && !staged.item_contained && staged.subtype == 0);
    // Emulate the game finishing construction: job gone, log moved into the
    // building, worker on another job. The completion read requires neither the
    // log's earlier availability nor an idle worker.
    workshop->stage = 3; workshop->flags.bits.exists = 1; workshop->jobs.clear();
    item().flags.bits.on_ground = false; item().flags.bits.in_job = false; item().flags.bits.in_building = true;
    item().specific_refs.clear();
    mock::contained.push_back(std::make_unique<df::buildingitemst>()); mock::contained.back()->item = &item();
    mock::contained.back()->use_mode = 2; workshop->contained_items.push_back(mock::contained.back().get());
    mock::units.front()->status.labors[df::unit_labor::CARPENTER] = false;
    mock::tick = 13212;
    const auto done = engine.progress(70, 42, progress);
    CHECK(done.present && done.carpenters && done.stage == 3 && done.max_stage == 3 && done.exists);
    CHECK(done.construct_jobs == 0 && done.jobs == 0 && done.contained == 1 && done.item_contained);
    CHECK(done.x1 == 14 && done.y2 == 16 && done.material == 420 && done.material_index == 163 && done.tick == 13212);
    // The historical receipt is unchanged by later state.
    CHECK(query(q).effect == reply.effect);
}
void progress_absence_and_identity() {
    const auto absent = engine.progress(70, 42, progress);
    CHECK(!absent.present && absent.next_building == 70 && absent.building == 70);
    auto *other = existing(5, 30, 30); other->stage = 1; other->maximum_stage = 1;
    const auto bed = engine.progress(5, 42, progress);
    CHECK(bed.present && !bed.carpenters && bed.subtype == -1);
    auto *mason = new df::building_workshopst(); mason->id = 6; mason->type = df::workshop_type::Masons;
    mason->x1 = 1; mason->x2 = 3; mason->y1 = 1; mason->y2 = 3; mason->centerx = 2; mason->centery = 2; mason->z = 2;
    world.buildings.all.push_back(mason);
    CHECK(!engine.progress(6, 42, progress).carpenters && engine.progress(6, 42, progress).subtype == 2);
    world.buildings.all.push_back(other); denied(progress_reply(5), 5); // duplicate registry entry
    world.buildings.all.pop_back(); mock::next_building = 6; denied(progress_reply(5), 5); // ID horizon
}
void footprint_and_registry_eligibility() {
    const std::vector<std::function<void()>> cases{
        [] { mock::paused = false; },
        [] { designation(14, 14).bits.flow_size = 1; },
        [] { block(16, 16).tiletype[0][0] = df::tiletype::Wall; },
        [] { designation(15, 16).bits.dig = df::tile_dig_designation::Default; },
        [] { designation(16, 14).bits.pile = 1; },
        [] { designation(14, 15).bits.smooth = 1; },
        [] { occupancy(16, 15).bits.unit = 1; },
        [] { occupancy(14, 16).bits.item = 1; },
        [] { existing(1, 16, 16); },
        [] { mock::free_tiles = false; },
        [] { mock::supported = false; },
        [] { mock::next_building = INT32_MAX; },
    };
    for (std::size_t i = 0; i < cases.size(); ++i) {
        reset(); cases[i](); ineligible();
    }
    // A zone room rectangle touching only one footprint corner is refused.
    reset();
    auto *zone = new df::building_civzonest(); zone->id = 2; zone->x1 = 0; zone->y1 = 0; zone->x2 = 14; zone->y2 = 14; zone->z = 2;
    zone->room.x = 13; zone->room.y = 13; zone->room.width = 2; zone->room.height = 2;
    world.buildings.all.push_back(zone); world.buildings.other.ANY_ZONE.push_back(zone);
    { auto c = observed(); CHECK(c.tiles[0].zone && c.tiles[0].building && !c.eligible()); }
    ineligible();
    reset(); zone = new df::building_civzonest(); zone->id = 2; zone->x1 = 0; zone->y1 = 0; zone->x2 = 2; zone->y2 = 2; zone->z = 2;
    zone->room.x = 16; zone->room.y = 16; zone->room.width = 1; zone->room.height = 1;
    world.buildings.all.push_back(zone); world.buildings.other.ANY_ZONE.push_back(zone);
    { auto c = observed(); CHECK(c.tiles[8].zone && !c.tiles[8].building && !c.free_footprint); }
    ineligible();
    // An unregistered zone pointer invalidates the whole capture.
    reset(); auto *orphan = new df::building_civzonest(); orphan->id = 9; world.buildings.other.ANY_ZONE.push_back(orphan);
    denied(read(), 5);
    // Hidden footprint cells carry no attributes and suppress helper calls.
    reset(); designation(16, 16).bits.hidden = 1;
    { auto c = observed(); CHECK(c.tiles[8].presence == ws::Presence::Hidden && !c.free_footprint && !c.supported); }
    CHECK(mock::helper_calls == 0 && mock::walk_calls == 0); ineligible();
}
void material_and_item_eligibility() {
    const std::vector<std::function<void()>> cases{
        [] { item().type = df::item_type::BOULDER; },
        [] { item().material = 0; item().material_index = 5; },
        [] { mock::materials[{420, 163}].flags.values.clear(); },
        [] { item().flags.bits.forbid = 1; },
        [] { item().flags.bits.dump = 1; },
        [] { item().flags.bits.in_job = 1; },
        [] { item().flags.bits.on_ground = 0; },
        [] { item().wear = 1; },
        [] { mock::walk_groups[{10, 11, 2}] = 2; },
        [] { designation(10, 11).bits.flow_size = 2; },
    };
    for (const auto &change : cases) { reset(); change(); ineligible(); }
    reset(); item().flags.bits.hidden = 1;
    { auto c = observed(); CHECK(c.item.presence == ws::Presence::Hidden); } ineligible();
    reset(); item().flags2.whole = 1; denied(read(), 5);
    reset(); world.items.all.clear();
    { auto c = observed(); CHECK(c.item.presence == ws::Presence::Missing); } ineligible();
    // A log inside the footprint is refused explicitly, even if the native item
    // occupancy bit is absent, so an eligible input can never make the exact
    // expected-after ground tile wrong by construction.
    for (const auto &[x, y, bit] : std::vector<std::tuple<int, int, bool>>{{15, 15, false}, {14, 16, false}, {16, 14, true}}) {
        reset(); occupancy(10, 11).bits.item = 0; item().pos = {x, y, 2}; mock::walk_groups[{x, y, 2}] = 1;
        occupancy(x, y).bits.item = bit;
        { auto c = observed(); CHECK(c.item_inside_footprint() && c.item.available() && !c.eligible());
          // Every other precondition holds, so the explicit exclusion is the
          // only reason an otherwise eligible overlapping input is refused.
          CHECK(c.paused && c.supported && c.census_complete && c.workers.size() == 1 && c.workers[0].eligible());
          bool floors = true; for (const auto &t : c.tiles) floors = floors && t.empty_floor();
          CHECK(floors == !bit && c.free_footprint); }
        ineligible();
    }
    // Adjacent but outside the footprint remains eligible.
    reset(); occupancy(10, 11).bits.item = 0; item().pos = {17, 15, 2}; mock::walk_groups[{17, 15, 2}] = 1;
    occupancy(17, 15).bits.item = 1;
    { auto c = observed(); CHECK(!c.item_inside_footprint() && c.eligible()); }
    { const auto q = prepared(); CHECK(commit(q).accepted && retained(q).phase == ws::Phase::Placed); }
    // Computed bookkeeping bits are permitted but remain witnessed.
    reset(); item().flags.whole |= ws::COMPUTED_ITEM_FLAGS;
    { auto c = observed(); CHECK(c.eligible() && c.item.other_flags == ws::COMPUTED_ITEM_FLAGS); }
}
void worker_census_eligibility() {
    const std::vector<std::function<void()>> cases{
        [] { mock::units.front()->status.labors[df::unit_labor::CARPENTER] = false; },
        [] { mock::walk_groups[{20, 20, 2}] = 7; },
        [] { mock::walk_groups[{20, 20, 2}] = 0; mock::walk_groups[{15, 15, 2}] = 0; },
        [] { mock::units.front()->adult = false; },
        [] { mock::units.front()->sane = false; },
        [] { designation(20, 20).bits.hidden = 1; },
        [] { mock::units.front()->citizen = false; },
        [] { mock::units.front()->alive = false; },
    };
    for (const auto &change : cases) { reset(); change(); ineligible(); }
    // The enabled CARPENTER bit alone is never availability: a busy, caged or
    // civilization-forbidden citizen keeps the bit but fails the precondition.
    const std::vector<std::function<void()>> unavailable{
        [] { mock::units.front()->has_current_job = true; },
        [] { mock::units.front()->caged = true; },
        [] { mock::units.front()->civ_permits_carpentry = false; },
    };
    for (const auto &change : unavailable) {
        reset(); change();
        { auto c = observed(); CHECK(c.workers.size() == 1 && c.workers[0].labor && c.workers[0].reachable);
          CHECK(!c.workers[0].eligible() && !c.eligible()); }
        ineligible();
        // The same transition after preparation is a stale witness, not a dispatch.
        reset(); const auto q = prepared(); change();
        CHECK(commit(q).accepted && retained(q).phase == ws::Phase::Refused);
        CHECK(retained(q).reason == ws::Reason::Stale && mock::writer_calls == 0);
    }
    // Non-carpenters remain observed facts but do not satisfy the precondition.
    reset(); add_unit(9, 21, 21, false);
    { auto c = observed(); CHECK(c.workers.size() == 2 && c.workers[1].id == 9 && !c.workers[1].labor && c.eligible()); }
    // More than 32 citizens is an explicit incomplete census, never a sample.
    reset(); for (int i = 0; i < 32; ++i) add_unit(100 + i, 30 + (i % 2), 30);
    { auto c = observed(); CHECK(!c.census_complete && c.workers.empty() && !c.eligible()); }
    ineligible();
    reset(); mock::units.front()->pos = {-1, 20, 2}; denied(read(), 5);
    reset(); world.units.active.push_back(world.units.active.front()); denied(read(), 5);
}
void stale_source_and_expiry_refusals() {
    const std::vector<std::function<void()>> changes{
        [] { item().pos = {11, 11, 2}; mock::walk_groups[{11, 11, 2}] = 1; },
        [] { item().material_index = 164; mock::materials[{420, 164}].flags.values = {df::material_flags::WOOD}; },
        [] { item().flags.bits.forbid = 1; },
        [] { mock::units.front()->status.labors[df::unit_labor::CARPENTER] = false; },
        [] { mock::units.front()->pos = {21, 20, 2}; mock::walk_groups[{21, 20, 2}] = 1; },
        [] { designation(14, 14).bits.dig = df::tile_dig_designation::Default; },
        [] { mock::next_job = 91; },
        [] { existing(1, 40, 40); },
        [] { mock::tick = 12346; },
    };
    for (const auto &change : changes) {
        reset(); const auto q = prepared(); change();
        const auto r = commit(q);
        CHECK(r.accepted && !r.unresolved && retained(q).phase == ws::Phase::Refused);
        CHECK(retained(q).reason == ws::Reason::Stale && mock::writer_calls == 0 && mock::allocator_calls == 0);
        CHECK(commit(q).effect == r.effect && mock::writer_calls == 0);
    }
    reset(); { const auto q = prepared(); CHECK(plugin_onstatechange(output, SC_MAP_LOADED) == CR_OK);
        CHECK(commit(q).accepted && retained(q).reason == ws::Reason::SourceChanged && mock::writer_calls == 0); }
    reset(); { const auto q = prepared(); CHECK(plugin_onstatechange(output, SC_UNPAUSED) == CR_OK);
        CHECK(commit(q).accepted && retained(q).reason == ws::Reason::Stale && mock::writer_calls == 0); }
    // Expiry uses only the local monotonic lifetime and is never renewed by replay.
    reset();
    {
        ws::Engine local(9); const auto sel = ws::Selection{ws::Kind::Carpenters, 42, {15, 15, 2}};
        const auto witness = local.inspect(sel, capture).witness(); const auto plan = ws::plan_digest(sel, witness);
        const auto &r = local.prepare("expiring", sel, witness, plan, 1000, capture);
        const auto token = r.token;
        CHECK(&local.prepare("expiring", sel, witness, plan, 60999, capture) == &r);
        const auto &out = local.commit("expiring", plan, token, 61000, capture,
            [](const ws::Capture &) { ++mock::writer_calls; }, verify);
        CHECK(out.phase == ws::Phase::Refused && out.reason == ws::Reason::Expired && mock::writer_calls == 0);
        // A regressed local clock is also expiry, never fresh permission.
        const auto &late = local.prepare("late", sel, witness, plan, 5000, capture);
        const auto late_token = late.token;
        const auto &regressed = local.commit("late", plan, late_token, 4999, capture,
            [](const ws::Capture &) { ++mock::writer_calls; }, verify);
        CHECK(regressed.phase == ws::Phase::Refused && regressed.reason == ws::Reason::Expired);
        CHECK(mock::writer_calls == 0 && !local.unresolved());
    }
    // Wrong token, plan or key never reaches a writer.
    reset(); { auto q = prepared(); auto bad = q; bad.token[0] ^= 1; denied(commit(bad), 7);
        bad = q; bad.plan[0] ^= 1; denied(commit(bad), 7); bad = q; bad.key = "other"; denied(commit(bad), 7);
        bad = q; bad.witness[0] ^= 1; bad.plan = ws::plan_digest(selection(&bad), bad.witness); bad.mask = PREPARE;
        bad.key = "fresh"; denied(call(PrepareWorkshop, bad), 6); CHECK(mock::writer_calls == 0); }
}
void writer_faults_and_uncertainty_fence() {
    for (auto fault : {mock::WriteFault::FalseBefore, mock::WriteFault::ThrowBefore,
            mock::WriteFault::BuildingOnly, mock::WriteFault::JobOnly}) {
        reset(); const auto q = prepared(); mock::write_fault = fault;
        const auto r = commit(q); indeterminate(q, r); CHECK(mock::writer_calls == 1);
        // Recovery is read-only; it never calls the writer or allocator again.
        mock::write_fault = mock::WriteFault::None;
        indeterminate(q, query(q)); indeterminate(q, commit(q)); indeterminate(q, cancel(q));
        CHECK(mock::writer_calls == 1 && mock::allocator_calls == 1);
        // A new key cannot dispatch while uncertainty is retained.
        auto other = candidate(); other.key = "workshop-002"; denied(call(PrepareWorkshop, other), 8);
    }
    for (auto fault : {mock::WriteFault::FalseAfter, mock::WriteFault::ThrowAfter}) {
        reset(); const auto q = prepared(); mock::write_fault = fault;
        const auto r = commit(q); CHECK(r.accepted && !r.unresolved && retained(q).phase == ws::Phase::Placed);
    }
    // The oracle requires the SDK's observed setting-occupancy and nine Planned
    // tiles; a different readback is safely retained as uncertainty.
    reset(); { const auto q = prepared(); mock::setting_occupancy = false; indeterminate(q, commit(q)); }
    // Design presence is an observed fact, not an assumption.
    reset(); { const auto q = prepared(); mock::needs_design = false; CHECK(commit(q).accepted);
        CHECK(retained(q).phase == ws::Phase::Placed && !retained(q).insertion->has_design); }
    // An unexpected relation, flag or second job is refused, not normalized away.
    const std::vector<std::function<void(df::building *, df::job *, df::item *)>> tampering{
        [](df::building *b, df::job *, df::item *) { b->relations.push_back(b); },
        [](df::building *b, df::job *, df::item *) { b->flags.bits.exists = 1; },
        [](df::building *, df::job *j, df::item *) { j->flags.bits.suspend = 1; },
        [](df::building *b, df::job *, df::item *) { b->stage = 1; },
        [](df::building *b, df::job *, df::item *) { b->mat_index = 1; },
        [](df::building *, df::job *j, df::item *) { j->job_type = df::job_type::DestroyBuilding; },
        [](df::building *, df::job *, df::item *i) { i->flags.bits.forbid = 1; },
        [](df::building *b, df::job *, df::item *) { b->jobs.push_back(b->jobs[0]); },
        [](df::building *, df::job *, df::item *) { occupancy(14, 14).bits.building = df::tile_building_occ::None; },
        // Still "occupied" in the capture, but not the SDK's stage-zero Planned mark.
        [](df::building *, df::job *, df::item *) { occupancy(16, 16).bits.building = df::tile_building_occ::Impassable; },
    };
    for (const auto &tamper : tampering) {
        reset(); const auto q = prepared(); mock::after_construct = tamper; indeterminate(q, commit(q));
    }
    // State changing while the insertion is being verified needs the final
    // complete recapture; a correct first capture alone cannot publish Placed.
    const std::vector<std::function<void(df::job *)>> races{
        [](df::job *) { mock::units.front()->status.labors[df::unit_labor::CARPENTER] = false; },
        [](df::job *) { mock::tick = 12399; },
        [](df::job *) { designation(16, 14).bits.smooth = 1; },
    };
    for (const auto &race : races) {
        reset(); const auto q = prepared(); mock::during_verify = race;
        indeterminate(q, commit(q)); CHECK(mock::writer_calls == 1);
    }
}
void recovery_frontier_and_revocation() {
    // A transient readback defect on the unchanged frontier is discharged only
    // by read-only query, with the original record and no second writer.
    const auto q = prepared(); df::building *added = nullptr;
    mock::after_construct = [&](df::building *b, df::job *, df::item *) { added = b; b->relations.push_back(b); };
    indeterminate(q, commit(q)); added->relations.clear();
    const auto healed = query(q); CHECK(healed.accepted && !healed.unresolved);
    CHECK(retained(q).phase == ws::Phase::Placed && mock::writer_calls == 1);
    // A later pause transition changes the frontier: uncertainty stays.
    reset(); { const auto p = prepared(); df::building *b2 = nullptr;
        mock::after_construct = [&](df::building *b, df::job *, df::item *) { b2 = b; b->relations.push_back(b); };
        indeterminate(p, commit(p)); b2->relations.clear(); CHECK(plugin_onstatechange(output, SC_PAUSED) == CR_OK);
        indeterminate(p, query(p)); CHECK(mock::writer_calls == 1); }
    // Credential rotation, revoked build permission or production marker after
    // allocation revokes this dispatch; the private allocation is destroyed.
    const std::vector<std::function<void()>> revocations{
        [] { setenv("DFMCP_WORKSHOP_TOKEN", std::string(32, 'r').c_str(), 1); },
        [] { unsetenv("DFMCP_WORKSHOP_ALLOW_BUILD"); },
        [] { setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "", 1); },
        [] { item().flags.bits.forbid = 1; },
    };
    for (const auto &revoke : revocations) {
        reset(); const auto p = prepared(); mock::after_allocation = revoke;
        indeterminate(p, commit(p)); CHECK(mock::writer_calls == 0 && world.buildings.all.empty());
        CHECK(df::live_buildings.empty());
    }
    reset(); { const auto p = prepared(); mock::allocation_failure = true; indeterminate(p, commit(p));
        CHECK(mock::writer_calls == 0); }
    reset(); { const auto p = prepared(); mock::resize_failure = true; indeterminate(p, commit(p));
        CHECK(mock::writer_calls == 0 && df::live_buildings.empty()); }
}
void replay_cancel_and_retention() {
    auto q = prepared(); const auto created = retained(q).created_ms; const auto bytes = retained(q).encode();
    auto again = q; again.mask = PREPARE; const auto replay = call(PrepareWorkshop, again);
    CHECK(replay.accepted && replay.replayed && replay.effect == bytes);
    // Replay never renews preparation lifetime or changes bytes.
    CHECK(retained(q).created_ms == created && engine.size() == 1);
    const auto c = cancel(q); CHECK(c.accepted && retained(q).phase == ws::Phase::Cancelled);
    CHECK(commit(q).effect == c.effect && mock::writer_calls == 0);
    // Unknown key query returns no record and no game access is needed for history.
    auto unknown = q; unknown.key = "never"; const auto none = query(unknown);
    CHECK(none.accepted && (none.mask & 2) == 0);
    // Bounded retention: the table never evicts.
    reset();
    for (std::size_t i = 0; i < ws::MAX_RECORDS; ++i) {
        wire::Request r; r.key = "k" + std::to_string(i); (void)prepared(r);
    }
    wire::Request overflow; overflow.key = "overflow"; overflow = candidate(overflow);
    denied(call(PrepareWorkshop, overflow), 5); CHECK(engine.size() == ws::MAX_RECORDS);
    // Oversized replies fail closed without leaking partial payloads.
    reset(); mock::oversized_reply = true; denied(read(), 5);
}
void record_bytes_are_closed() {
    const auto q = prepared(); const auto r = commit(q); const auto &record = retained(q);
    CHECK(r.effect.substr(0, 8) == "DFMWR020" && record.before.encode().substr(0, 8) == "DFMWC020");
    CHECK(record.insertion->encode().substr(0, 8) == "DFMWI020" && record.insertion->encode().size() == 77);
    CHECK(ws::token_for(q.key, q.plan) == record.token && record.token.size() == 16);
    // Domains differ from furniture/1.19 even for identical inputs.
    CHECK(ws::hash("dfmcp-workshop-plan/1", "x") != ws::hash("dfmcp-build-plan/1", "x"));
    auto tampered = record; tampered.insertion->material_index ^= 1;
    bool refused = false; try { (void)tampered.encode(); } catch (const ws::Failure &) { refused = true; }
    CHECK(refused);
    tampered = record; tampered.after->tiles[3].occupied = false; refused = false;
    try { (void)tampered.encode(); } catch (const ws::Failure &) { refused = true; }
    CHECK(refused);
}

std::string hex(const std::string &bytes) {
    std::ostringstream out;
    for (unsigned char c : bytes) out << std::hex << std::setw(2) << std::setfill('0') << static_cast<int>(c);
    return out.str();
}
int vectors() {
    reset(); const auto q = prepared(); const auto prepared_record = retained(q).encode();
    const auto placed = commit(q).effect; const auto staged = engine.progress(70, 42, progress).encode();
    auto *workshop = placed_workshop(); workshop->stage = 3; workshop->flags.bits.exists = 1; workshop->jobs.clear();
    mock::contained.push_back(std::make_unique<df::buildingitemst>()); mock::contained.back()->item = &item();
    workshop->contained_items.push_back(mock::contained.back().get()); mock::tick = 13212;
    const auto complete = engine.progress(70, 42, progress).encode();
    reset(); const auto absent = engine.progress(70, 42, progress).encode();
    reset(); const auto u = prepared(); mock::write_fault = mock::WriteFault::JobOnly;
    const auto uncertain = commit(u).effect;
    reset(); const auto c = prepared(); const auto cancelled = cancel(c).effect;
    reset(); const auto s = prepared(); mock::tick = 12346; const auto refused = commit(s).effect;
    std::cout << "{\"capture\":\"" << hex(retained(q).before.encode()) << "\",\"plan\":\"" << hex(q.plan)
        << "\",\"token\":\"" << hex(q.token) << "\",\"prepared\":\"" << hex(prepared_record)
        << "\",\"placed\":\"" << hex(placed) << "\",\"indeterminate\":\"" << hex(uncertain)
        << "\",\"cancelled\":\"" << hex(cancelled) << "\",\"refused\":\"" << hex(refused)
        << "\",\"progress_stage0\":\"" << hex(staged) << "\",\"progress_complete\":\"" << hex(complete)
        << "\",\"progress_absent\":\"" << hex(absent) << "\"}\n";
    cleanup(); return 0;
}
} // namespace

int main(int argc, char **argv) {
    if (argc == 2 && std::strcmp(argv[1], "--vectors") == 0) {
        try { current_group = "vectors"; return vectors(); }
        catch (const std::exception &e) { std::cerr << e.what() << "\n"; cleanup(); return 1; }
    }
    try {
        run("rpc_manifest", rpc_manifest);
        run("environment_and_profile_gates", environment_and_profile_gates);
        run("exact_request_shapes", exact_request_shapes);
        run("positive_construction_and_completion", positive_construction_and_completion);
        run("progress_absence_and_identity", progress_absence_and_identity);
        run("footprint_and_registry_eligibility", footprint_and_registry_eligibility);
        run("material_and_item_eligibility", material_and_item_eligibility);
        run("worker_census_eligibility", worker_census_eligibility);
        run("stale_source_and_expiry_refusals", stale_source_and_expiry_refusals);
        run("writer_faults_and_uncertainty_fence", writer_faults_and_uncertainty_fence);
        run("recovery_frontier_and_revocation", recovery_frontier_and_revocation);
        run("replay_cancel_and_retention", replay_cancel_and_retention);
        run("record_bytes_are_closed", record_bytes_are_closed);
    } catch (const std::exception &e) {
        std::cerr << e.what() << "\n"; cleanup(); return 1;
    }
    cleanup();
    std::cout << "{\"scenarios\":" << groups.size() << ",\"assertions\":" << checks << ",\"groups\":[";
    for (std::size_t i = 0; i < groups.size(); ++i) std::cout << (i ? "," : "") << "\"" << groups[i] << "\"";
    std::cout << "]}\n";
    return 0;
}
