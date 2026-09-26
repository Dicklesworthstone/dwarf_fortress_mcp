#include "../common/build_placement.h"
#include <chrono>
#include <cstdlib>
#include <memory>
#include <set>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "TileTypes.h"
#include "modules/Buildings.h"
#include "modules/Job.h"
#include "modules/Maps.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/building_civzonest.h"
#include "df/building_type.h"
#include "df/general_ref.h"
#include "df/general_ref_type.h"
#include "df/global_objects.h"
#include "df/item.h"
#include "df/item_type.h"
#include "df/job.h"
#include "df/job_item_ref.h"
#include "df/job_list_link.h"
#include "df/job_role_type.h"
#include "df/job_type.h"
#include "df/map_block.h"
#include "df/specific_ref.h"
#include "df/specific_ref_type.h"
#include "df/tile_building_occ.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_shape.h"
#include "df/world.h"
#include "DfmcpBuildV1_19.pb.h"

using namespace DFHack;
namespace wire = dfmcp::build::v1_19;
namespace bp = dfmcp_build;
DFHACK_PLUGIN("dfmcp_build_v1_19");
namespace {
std::uint64_t monotonic_ms() {
    const auto value = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now().time_since_epoch()).count();
    bp::require(value >= 0, 5); return static_cast<std::uint64_t>(value);
}
bp::Engine engine(monotonic_ms() | 1);
constexpr std::size_t MAX_REQUEST_BYTES = 2048, MAX_REPLY_BYTES = 8192;
constexpr std::size_t MAX_REFS = 4096, MAX_JOBS = 65536, MAX_ITEMS = 1048576;
bool enabled(const char *name) {
    const char *value = std::getenv(name); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(19); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    bp::require(in->ByteSizeLong() <= MAX_REQUEST_BYTES && in->IsInitialized()
        && in->GetReflection()->GetUnknownFields(*in).empty());
    bp::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    bp::require(in->protocol_major() == 1 && in->protocol_minor() == 19, 2);
    bp::require(std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") == nullptr, 1);
    bp::require(enabled("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19"), 1);
    const char *configured = std::getenv("DFMCP_BUILD_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &provided = in->bearer_token();
    bp::require(expected.size() >= 32 && expected.size() <= 256
        && provided.size() >= 32 && provided.size() <= 256, 1);
    std::size_t difference = expected.size() ^ provided.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0;
        const unsigned char b = i < provided.size() ? provided[i] : 0;
        difference |= a ^ b;
    }
    bp::require(difference == 0, 1);
}
void manifest(wire::Reply *out) {
    bp::require(engine.available(), 5);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const char *version = Version::dfhack_version(); bp::require(version != nullptr, 5);
    const std::string dfhack(version); bp::require(bp::utf8(df, 128) && bp::utf8(dfhack, 128), 5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void require_fortress() {
    bp::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world && Maps::IsValid(), 4);
}
df::coord native_pos(bp::Coord p) {
    (void)p.encode(); return df::coord(static_cast<std::int16_t>(p.x),
        static_cast<std::int16_t>(p.y), static_cast<std::int16_t>(p.z));
}
bp::Kind item_kind(df::item_type type) {
    switch (type) {
        case df::item_type::BED: return bp::Kind::Bed;
        case df::item_type::CHAIR: return bp::Kind::Chair;
        case df::item_type::TABLE: return bp::Kind::Table;
        default: return bp::Kind::Other;
    }
}
df::building_type building_kind(bp::Kind kind) {
    switch (kind) {
        case bp::Kind::Bed: return df::building_type::Bed;
        case bp::Kind::Chair: return df::building_type::Chair;
        case bp::Kind::Table: return df::building_type::Table;
        default: throw bp::Failure(3);
    }
}
unsigned shape_value(df::tiletype_shape shape) {
    switch (shape) {
        case df::tiletype_shape::EMPTY: return 1;
        case df::tiletype_shape::WALL: return 2;
        case df::tiletype_shape::FLOOR: return 3;
        case df::tiletype_shape::RAMP: return 4;
        case df::tiletype_shape::RAMP_TOP: return 5;
        case df::tiletype_shape::STAIR_UP: return 6;
        case df::tiletype_shape::STAIR_DOWN: return 7;
        case df::tiletype_shape::STAIR_UPDOWN: return 8;
        default: return 0;
    }
}
// Validate the complete bounded registry before using its count or proving an
// empty footprint. Zones and stockpiles count as overlaps, including their holes.
const std::vector<df::building *> &buildings() {
    bp::require(df::global::building_next_id && *df::global::building_next_id >= 0, 5);
    const auto &all = df::global::world->buildings.all;
    bp::require(all.size() <= bp::MAX_BUILDINGS, 5); std::set<std::int32_t> ids;
    for (const auto *b : all) {
        bp::require(b && b->id >= 0 && b->id < *df::global::building_next_id
            && ids.insert(b->id).second && b->x1 >= 0 && b->y1 >= 0 && b->z >= 0
            && b->x2 >= b->x1 && b->y2 >= b->y1 && b->x2 < 32768 && b->y2 < 32768 && b->z < 32768, 5);
    }
    return all;
}
bool no_zone_at(bp::Coord p, const std::vector<df::building *> &all) {
    const auto &zones = df::global::world->buildings.other.ANY_ZONE;
    bp::require(zones.size() <= bp::MAX_BUILDINGS, 5);
    const std::set<const df::building *> registered(all.begin(), all.end());
    std::set<std::int32_t> ids; bool clear = true;
    for (auto *zone : zones) {
        bp::require(zone && registered.count(zone) && ids.insert(zone->id).second
            && zone->getType() == df::building_type::Civzone, 5);
        const auto &room = zone->room;
        bp::require(room.x >= 0 && room.y >= 0 && room.width >= 0 && room.height >= 0
            && static_cast<std::int64_t>(room.x) + room.width <= 32768
            && static_cast<std::int64_t>(room.y) + room.height <= 32768, 5);
        // DFHack uses room extents, not base building coordinates, for automatic
        // zone attachment. Reject the whole extent rectangle without reading an
        // unbounded native extent allocation; holes remain conservatively blocked.
        if (zone->z == static_cast<std::int32_t>(p.z) && room.x <= static_cast<std::int32_t>(p.x)
            && static_cast<std::int64_t>(p.x) < static_cast<std::int64_t>(room.x) + room.width
            && room.y <= static_cast<std::int32_t>(p.y)
            && static_cast<std::int64_t>(p.y) < static_cast<std::int64_t>(room.y) + room.height) clear = false;
    }
    return clear;
}
bp::Tile tile(bp::Coord p, const std::vector<df::building *> &all) {
    bp::Tile out;
    auto *block = Maps::getTileBlock(static_cast<std::int32_t>(p.x),
        static_cast<std::int32_t>(p.y), static_cast<std::int32_t>(p.z));
    if (!block) return out;
    const auto x = p.x & 15, y = p.y & 15; const auto designation = block->designation[x][y];
    if (designation.bits.hidden) { out.presence = bp::Presence::Hidden; return out; }
    const auto type = block->tiletype[x][y];
    bp::require(static_cast<int>(type) >= 0 && is_valid_enum_item(type), 5);
    out.presence = bp::Presence::Visible; out.tiletype = static_cast<std::uint32_t>(type);
    out.shape = static_cast<unsigned char>(shape_value(tileShape(type)));
    out.liquid = static_cast<unsigned char>(designation.bits.flow_size);
    out.dig = static_cast<unsigned char>(designation.bits.dig);
    auto occupancy = block->occupancy[x][y]; out.occupied = occupancy.bits.building != df::tile_building_occ::None;
    occupancy.bits.building = df::tile_building_occ::None; out.occupancy_other = occupancy.whole;
    for (const auto *b : all) {
        if (b->z == static_cast<std::int32_t>(p.z) && b->x1 <= static_cast<std::int32_t>(p.x)
            && b->x2 >= static_cast<std::int32_t>(p.x) && b->y1 <= static_cast<std::int32_t>(p.y)
            && b->y2 >= static_cast<std::int32_t>(p.y)) {
            // Overlapping zone bounds are ambiguous and conservatively refused.
            bp::require(!out.building, 4); out.building = static_cast<std::uint32_t>(b->id);
        }
    }
    return out;
}
bp::Item item_capture(std::uint32_t id, const std::array<std::uint32_t, 3> &dimensions,
    const std::vector<df::building *> &all) {
    bp::Item out; bp::require(id < INT32_MAX && df::global::world->items.all.size() <= MAX_ITEMS, 5);
    auto *item = df::item::find(static_cast<std::int32_t>(id));
    if (!item) return out;
    bp::require(item->id >= 0 && static_cast<std::uint32_t>(item->id) == id, 5);
    if (item->flags.bits.hidden) { out.presence = bp::Presence::Hidden; return out; }
    if (item->pos.x < 0 || item->pos.y < 0 || item->pos.z < 0) return out;
    const bp::Coord p{static_cast<std::uint32_t>(item->pos.x), static_cast<std::uint32_t>(item->pos.y),
        static_cast<std::uint32_t>(item->pos.z)};
    bp::require(p.x < dimensions[0] && p.y < dimensions[1] && p.z < dimensions[2], 5);
    const auto ground = tile(p, all);
    if (ground.presence != bp::Presence::Visible) { out.presence = ground.presence; return out; }
    // This second flag word is outside the frozen capture encoding. Refuse it
    // rather than OR-folding fields and creating indistinguishable witnesses.
    bp::require(item->flags2.whole == 0 && item->general_refs.size() <= MAX_REFS
        && item->specific_refs.size() <= MAX_REFS - item->general_refs.size(), 5);
    out.presence = bp::Presence::Visible; out.pos = p; out.ground = ground;
    const auto type = item->getType(); bp::require(static_cast<int>(type) >= 0 && is_valid_enum_item(type), 5);
    out.kind = item_kind(type); out.native_type = static_cast<std::uint32_t>(type);
    out.subtype = item->getSubtype(); out.material = item->getMaterial(); out.material_index = item->getMaterialIndex();
    const auto quality = item->getQuality(), wear = item->getWear();
    bp::require(quality >= 0 && wear >= 0, 5); out.quality = quality; out.wear = wear;
    auto flags = item->flags; out.on_ground = flags.bits.on_ground; out.in_job = flags.bits.in_job;
    flags.bits.on_ground = false; flags.bits.in_job = false; out.other_flags = flags.whole;
    for (const auto *ref : item->general_refs) { bp::require(ref, 5); ++out.other_refs; }
    for (const auto *ref : item->specific_refs) {
        bp::require(ref, 5);
        if (ref->type == df::specific_ref_type::JOB) {
            bp::require(ref->data.job && ref->data.job->id >= 0 && out.jobs.size() < 8, 5);
            out.jobs.push_back(static_cast<std::uint32_t>(ref->data.job->id));
        } else ++out.other_refs;
    }
    std::sort(out.jobs.begin(), out.jobs.end()); (void)out.encode(); return out;
}
bp::Capture capture(const bp::Selection &selected) {
    (void)selected.encode(); require_fortress();
    std::int32_t x = 0, y = 0, z = 0; Maps::getTileSize(x, y, z);
    bp::require(x > 0 && y > 0 && z > 0 && x <= 32768 && y <= 32768 && z <= 32768
        && selected.target.x + 1 < static_cast<std::uint32_t>(x)
        && selected.target.y + 1 < static_cast<std::uint32_t>(y) && selected.target.z < static_cast<std::uint32_t>(z), 4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    bp::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX
        && df::global::job_next_id && *df::global::job_next_id >= 0, 5);
    const auto &all = buildings(); bp::Capture out;
    out.selection = selected; out.dimensions = {static_cast<std::uint32_t>(x), static_cast<std::uint32_t>(y), static_cast<std::uint32_t>(z)};
    out.tick = static_cast<std::uint64_t>(year) * 403200 + tick; out.site = static_cast<std::uint32_t>(site);
    out.folder = World::ReadWorldFolder(); out.paused = World::ReadPauseState();
    out.next_building = static_cast<std::uint32_t>(*df::global::building_next_id);
    out.next_job = static_cast<std::uint32_t>(*df::global::job_next_id); out.building_count = static_cast<std::uint32_t>(all.size());
    std::size_t index = 0;
    for (auto ty = selected.target.y - 1; ty <= selected.target.y + 1; ++ty)
    for (auto tx = selected.target.x - 1; tx <= selected.target.x + 1; ++tx)
        out.tiles[index++] = tile({tx, ty, selected.target.z}, all);
    out.item = item_capture(selected.item, out.dimensions, all);
    // Never ask helper APIs to inspect hidden or missing terrain. The exact
    // target/halo tags remain visible evidence of why preparation is ineligible.
    bool visible = true;
    for (const auto &t : out.tiles) visible = visible && t.presence == bp::Presence::Visible;
    if (visible) {
        const auto pos = native_pos(selected.target); const df::coord2d size(1, 1);
        const auto *block = Maps::getTileBlock(pos.x, pos.y, pos.z);
        const auto &des = block->designation[pos.x & 15][pos.y & 15];
        const bool zones_clear = no_zone_at(selected.target, all);
        out.free_tile = zones_clear && !out.tiles[4].building && !des.bits.pile && !des.bits.smooth
            && Buildings::checkFreeTiles(pos, size, nullptr, false, false, false, false);
        out.supported = Buildings::hasSupport(pos, size);
    }
    return out;
}
void place(const bp::Capture &before) {
    require_fortress(); bp::require(enabled("DFMCP_BUILD_ALLOW_PLACE"), 1);
    bp::require(df::global::plotinfo != nullptr, 5);
    auto building = std::unique_ptr<df::building>(Buildings::allocInstance(native_pos(before.selection.target),
        building_kind(before.selection.kind), -1, -1));
    bp::require(building && building->id == -1 && Buildings::setSize(building.get(), df::coord2d(1, 1), 0), 5);
    std::vector<df::item *> selected_items;
    selected_items.reserve(1);
    auto *selected = df::item::find(static_cast<std::int32_t>(before.selection.item));
    bp::require(selected, 6); selected_items.push_back(selected);
    // Allocation-only helper calls can fail. Revalidate everything again after
    // them and immediately before handing ownership to the effectful API.
    auto current = capture(before.selection); current.generation = before.generation; current.sequence = before.sequence;
    bp::require(engine.generation() == before.generation && engine.sequence() == before.sequence + 1
        && current.encode() == before.encode() && current.eligible(), 6);
    bp::require(std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") == nullptr
        && enabled("DFMCP_ALLOW_UNADMITTED_BUILD_V1_19") && enabled("DFMCP_BUILD_ALLOW_PLACE"), 1);
    // constructWithItems may throw after any registry/job/item/occupancy link.
    // Once entered, deleting this pointer could corrupt the game. Retain Unknown
    // even on false return; only independent readback can publish Placed.
    auto *native = building.release();
    (void)Buildings::constructWithItems(native, std::move(selected_items));
}
bp::Insertion verify(const bp::Capture &before) {
    require_fortress(); const auto &all = buildings(); df::building *building = nullptr;
    for (auto *candidate : all) if (static_cast<std::uint32_t>(candidate->id) == before.next_building) building = candidate;
    bp::require(building && building->getType() == building_kind(before.selection.kind)
        && building->x1 == static_cast<std::int32_t>(before.selection.target.x)
        && building->x2 == building->x1 && building->centerx == building->x1
        && building->y1 == static_cast<std::int32_t>(before.selection.target.y)
        && building->y2 == building->y1 && building->centery == building->y1
        && building->z == static_cast<std::int32_t>(before.selection.target.z)
        && building->mat_type == before.item.material && building->mat_index == before.item.material_index
        && building->jobs.size() == 1 && building->general_refs.empty() && building->specific_refs.empty()
        && building->relations.empty(), 5);
    const auto pos = native_pos(before.selection.target);
    const auto *target = Maps::getTileBlock(pos.x, pos.y, pos.z);
    bp::require(target && target->occupancy[pos.x & 15][pos.y & 15].bits.building == df::tile_building_occ::Planned, 5);
    auto *job = building->jobs[0]; bp::require(job && job->id >= 0
        && static_cast<std::uint32_t>(job->id) == before.next_job, 5);
    std::set<std::int32_t> job_ids; bool found = false;
    auto *previous = &df::global::world->jobs.list;
    for (auto *link = df::global::world->jobs.list.next; link; link = link->next) {
        bp::require(job_ids.size() < MAX_JOBS && link->item && link->item->id >= 0
            && link->prev == previous && link->item->id < *df::global::job_next_id
            && job_ids.insert(link->item->id).second && link->item->list_link == link, 5);
        if (link->item == job) { bp::require(job->list_link == link, 5); found = true; }
        previous = link;
    }
    bp::require(found && job->job_type == df::job_type::ConstructBuilding
        && job->pos.x == building->centerx && job->pos.y == building->centery && job->pos.z == building->z
        && job->mat_type == building->mat_type && job->mat_index == building->mat_index
        && job->flags.whole == 0 && job->job_items.elements.empty() && job->items.size() == 1 && job->specific_refs.empty()
        && job->general_refs.size() == 1 && job->general_refs[0]
        && job->general_refs[0]->getType() == df::general_ref_type::BUILDING_HOLDER
        && job->general_refs[0]->getBuilding() == building && Job::getHolder(job) == building, 5);
    auto *item = df::item::find(static_cast<std::int32_t>(before.selection.item));
    const auto *attachment = job->items[0];
    bp::require(item && attachment && attachment->item == item
        && attachment->role == df::job_role_type::Hauled && attachment->job_item_idx == -1
        && attachment->flags.whole == 0 && item->general_refs.empty() && item->specific_refs.size() == 1
        && item->specific_refs[0] && item->specific_refs[0]->type == df::specific_ref_type::JOB
        && item->specific_refs[0]->data.job == job && item->flags.bits.in_job, 5);
    const auto stage = building->getBuildStage(), max_stage = building->getMaxBuildStage();
    bp::require(stage == 0 && max_stage >= 1 && max_stage <= 32, 5);
    bp::Insertion proof; proof.building = before.next_building; proof.job = before.next_job;
    proof.item = before.selection.item; proof.kind = before.selection.kind; proof.pos = before.selection.target;
    proof.material = building->mat_type; proof.material_index = building->mat_index;
    proof.stage = stage; proof.max_stage = max_stage;
    proof.linked = true; proof.construct_job = true; proof.exact_item_link = true; proof.suspended = job->flags.bits.suspend;
    return proof;
}
unsigned shape(const wire::Request *in) {
    return (in->has_kind() ? 1 : 0) | (in->has_item_id() ? 2 : 0) | (in->has_x() ? 4 : 0)
        | (in->has_y() ? 8 : 0) | (in->has_z() ? 16 : 0) | (in->has_idempotency_key() ? 32 : 0)
        | (in->has_expected_witness() ? 64 : 0) | (in->has_plan_digest() ? 128 : 0) | (in->has_prepare_token() ? 256 : 0);
}
bp::Selection selection(const wire::Request *in) {
    bp::require(in->kind() >= 1 && in->kind() <= 3);
    bp::Selection out{static_cast<bp::Kind>(in->kind()), in->item_id(), {in->x(), in->y(), in->z()}};
    (void)out.encode(); return out;
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out,
    unsigned expected_shape, bool placing, Body body) {
    try {
        authorize(in, out); bp::require(shape(in) == expected_shape);
        if (placing) bp::require(enabled("DFMCP_BUILD_ALLOW_PLACE"), 1);
        manifest(out); body(); out->set_unresolved(engine.unresolved());
        out->set_retained_records(static_cast<std::uint32_t>(engine.size()));
        out->set_failure_code(0); out->set_accepted(true); bp::require(out->ByteSizeLong() <= MAX_REPLY_BYTES, 5);
        return CR_OK;
    } catch (const bp::Failure &e) {
        try { empty_reply(in, out, e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in, out, 5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 0, false, [] {});
}
command_result ReadPlacement(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 31, false, [&] { out->set_observation(engine.inspect(selection(in), capture).encode()); });
}
command_result PreparePlacement(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 255, true, [&] {
        const bool replay = engine.query(in->idempotency_key(), in->plan_digest()) != nullptr;
        const auto &record = engine.prepare(in->idempotency_key(), selection(in), in->expected_witness(),
            in->plan_digest(), monotonic_ms(), capture);
        out->set_effect_record(record.encode()); out->set_replayed(replay);
    });
}
command_result CommitPlacement(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 416, true, [&] {
        out->set_effect_record(engine.commit(in->idempotency_key(), in->plan_digest(), in->prepare_token(),
            monotonic_ms(), capture, place, verify).encode());
    });
}
command_result QueryPlacement(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 160, false, [&] {
        if (const auto *record = engine.query(in->idempotency_key(), in->plan_digest())) out->set_effect_record(record->encode());
    });
}
command_result CancelPlacement(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 416, false, [&] {
        out->set_effect_record(engine.cancel(in->idempotency_key(), in->plan_digest(), in->prepare_token()).encode());
    });
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { engine.change_source(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_MAP_LOADED || event == SC_MAP_UNLOADED || event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) engine.change_source();
    else if (event == SC_PAUSED || event == SC_UNPAUSED) engine.interrupt();
    return CR_OK;
}
// Zero flags: DFHack's suspended dispatch owns the entire read/prepare/commit.
// No native pointer, scheduled work, timer or effect is retained across calls.
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ReadPlacement", ReadPlacement, 0);
    service->addFunction("PreparePlacement", PreparePlacement, 0); service->addFunction("CommitPlacement", CommitPlacement, 0);
    service->addFunction("QueryPlacement", QueryPlacement, 0); service->addFunction("CancelPlacement", CancelPlacement, 0);
    return service;
}
