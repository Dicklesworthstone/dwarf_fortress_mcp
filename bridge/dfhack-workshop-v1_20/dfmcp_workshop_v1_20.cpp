#include "../common/workshop_construction.h"
#include <algorithm>
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
#include "modules/Materials.h"
#include "modules/Units.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/building_civzonest.h"
#include "df/building_type.h"
#include "df/building_workshopst.h"
#include "df/buildingitemst.h"
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
#include "df/material.h"
#include "df/material_flags.h"
#include "df/specific_ref.h"
#include "df/specific_ref_type.h"
#include "df/tile_building_occ.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_shape.h"
#include "df/unit.h"
#include "df/unit_labor.h"
#include "df/workshop_type.h"
#include "df/world.h"
#include "DfmcpWorkshopV1_20.pb.h"

using namespace DFHack;
namespace wire = dfmcp::workshop::v1_20;
namespace ws = dfmcp_workshop;
DFHACK_PLUGIN("dfmcp_workshop_v1_20");
namespace {
std::uint64_t monotonic_ms() {
    const auto value = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now().time_since_epoch()).count();
    ws::require(value >= 0, 5); return static_cast<std::uint64_t>(value);
}
ws::Engine engine(monotonic_ms() | 1);
constexpr std::size_t MAX_REQUEST_BYTES = 2048, MAX_REPLY_BYTES = 8192;
constexpr std::size_t MAX_REFS = 4096, MAX_JOBS = 65536, MAX_ITEMS = 1048576, MAX_UNITS = 8192;
constexpr std::size_t MAX_BUILDING_JOBS = 8, MAX_CONTAINED = 64;
bool enabled(const char *name) {
    const char *value = std::getenv(name); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(20); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize_credential(const std::string &provided) {
    const char *configured = std::getenv("DFMCP_WORKSHOP_TOKEN");
    const std::string_view expected = configured ? configured : "";
    ws::require(expected.size() >= 32 && expected.size() <= 256
        && provided.size() >= 32 && provided.size() <= 256, 1);
    std::size_t difference = expected.size() ^ provided.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0;
        const unsigned char b = i < provided.size() ? provided[i] : 0;
        difference |= a ^ b;
    }
    ws::require(difference == 0, 1);
}
bool development_profile() {
    return std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") == nullptr
        && enabled("DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    ws::require(static_cast<std::size_t>(in->ByteSize()) <= MAX_REQUEST_BYTES && in->IsInitialized()
        && in->GetReflection()->GetUnknownFields(*in).empty());
    ws::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    ws::require(in->protocol_major() == 1 && in->protocol_minor() == 20, 2);
    ws::require(development_profile(), 1);
    authorize_credential(in->bearer_token());
}
void manifest(wire::Reply *out) {
    ws::require(engine.available(), 5);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const char *version = Version::dfhack_version(); ws::require(version != nullptr, 5);
    const std::string dfhack(version); ws::require(ws::utf8(df, 128) && ws::utf8(dfhack, 128), 5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void require_fortress() {
    ws::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world && Maps::IsValid(), 4);
}
df::coord native_pos(ws::Coord p) {
    (void)p.encode(); return df::coord(static_cast<std::int16_t>(p.x),
        static_cast<std::int16_t>(p.y), static_cast<std::int16_t>(p.z));
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
struct Clock { std::uint64_t tick = 0; std::uint32_t site = 0; std::array<std::uint32_t, 3> dimensions{}; std::string folder; };
Clock clock_identity() {
    std::int32_t x = 0, y = 0, z = 0; Maps::getTileSize(x, y, z);
    ws::require(x > 0 && y > 0 && z > 0 && x <= 32768 && y <= 32768 && z <= 32768, 4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    ws::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX, 5);
    Clock out; out.tick = static_cast<std::uint64_t>(year) * 403200 + tick; out.site = static_cast<std::uint32_t>(site);
    out.dimensions = {static_cast<std::uint32_t>(x), static_cast<std::uint32_t>(y), static_cast<std::uint32_t>(z)};
    out.folder = World::ReadWorldFolder(); return out;
}
// Validate the complete bounded registry before using its count, proving an
// empty footprint or locating a historical building ID.
const std::vector<df::building *> &buildings() {
    ws::require(df::global::building_next_id && *df::global::building_next_id >= 0, 5);
    const auto &all = df::global::world->buildings.all;
    ws::require(all.size() <= ws::MAX_BUILDINGS, 5); std::set<std::int32_t> ids;
    for (const auto *b : all) {
        ws::require(b && b->id >= 0 && b->id < *df::global::building_next_id
            && ids.insert(b->id).second && b->x1 >= 0 && b->y1 >= 0 && b->z >= 0
            && b->x2 >= b->x1 && b->y2 >= b->y1 && b->x2 < 32768 && b->y2 < 32768 && b->z < 32768, 5);
    }
    return all;
}
// The complete bounded zone registry. DFHack attaches new buildings to zones
// containing their center; any zone room rectangle touching the footprint is
// refused, extent holes included, so that ancillary relation write cannot occur.
const std::vector<df::building_civzonest *> &zones(const std::vector<df::building *> &all) {
    const auto &values = df::global::world->buildings.other.ANY_ZONE;
    ws::require(values.size() <= all.size(), 5);
    const std::set<const df::building *> registered(all.begin(), all.end());
    std::set<std::int32_t> ids;
    for (auto *zone : values) {
        ws::require(zone && registered.count(zone) && ids.insert(zone->id).second
            && zone->getType() == df::building_type::Civzone, 5);
        const auto &room = zone->room;
        ws::require(room.x >= 0 && room.y >= 0 && room.width >= 0 && room.height >= 0
            && static_cast<std::int64_t>(room.x) + room.width <= 32768
            && static_cast<std::int64_t>(room.y) + room.height <= 32768, 5);
    }
    return values;
}
bool in_zone(ws::Coord p, const std::vector<df::building_civzonest *> &all_zones) {
    for (const auto *zone : all_zones) {
        const auto &room = zone->room;
        if (zone->z == static_cast<std::int32_t>(p.z) && room.x <= static_cast<std::int32_t>(p.x)
            && static_cast<std::int64_t>(p.x) < static_cast<std::int64_t>(room.x) + room.width
            && room.y <= static_cast<std::int32_t>(p.y)
            && static_cast<std::int64_t>(p.y) < static_cast<std::int64_t>(room.y) + room.height) return true;
    }
    return false;
}
ws::Tile tile(ws::Coord p, const std::vector<df::building *> &all,
    const std::vector<df::building_civzonest *> &all_zones) {
    ws::Tile out;
    auto *block = Maps::getTileBlock(static_cast<std::int32_t>(p.x),
        static_cast<std::int32_t>(p.y), static_cast<std::int32_t>(p.z));
    if (!block) return out;
    const auto x = p.x & 15, y = p.y & 15; const auto designation = block->designation[x][y];
    if (designation.bits.hidden) { out.presence = ws::Presence::Hidden; return out; }
    const auto type = block->tiletype[x][y];
    ws::require(static_cast<int>(type) >= 0 && is_valid_enum_item(type), 5);
    out.presence = ws::Presence::Visible; out.tiletype = static_cast<std::uint32_t>(type);
    out.shape = static_cast<unsigned char>(shape_value(tileShape(type)));
    out.liquid = static_cast<unsigned char>(designation.bits.flow_size);
    out.dig = static_cast<unsigned char>(designation.bits.dig);
    out.pile = designation.bits.pile; out.smooth = static_cast<unsigned char>(designation.bits.smooth);
    auto occupancy = block->occupancy[x][y]; out.occupied = occupancy.bits.building != df::tile_building_occ::None;
    occupancy.bits.building = df::tile_building_occ::None; out.occupancy_other = occupancy.whole;
    for (const auto *b : all) {
        if (b->z == static_cast<std::int32_t>(p.z) && b->x1 <= static_cast<std::int32_t>(p.x)
            && b->x2 >= static_cast<std::int32_t>(p.x) && b->y1 <= static_cast<std::int32_t>(p.y)
            && b->y2 >= static_cast<std::int32_t>(p.y)) {
            // Overlapping registered bounds are ambiguous and conservatively refused.
            ws::require(!out.building, 4); out.building = static_cast<std::uint32_t>(b->id);
        }
    }
    out.zone = in_zone(p, all_zones);
    return out;
}
bool plant_wood(df::item *item) {
    MaterialInfo info(item->getMaterial(), item->getMaterialIndex());
    return info.isPlant() && info.material && info.material->flags.is_set(df::material_flags::WOOD);
}
ws::Item item_capture(std::uint32_t id, const std::array<std::uint32_t, 3> &dimensions,
    const std::vector<df::building *> &all, const std::vector<df::building_civzonest *> &all_zones) {
    ws::Item out; ws::require(id < INT32_MAX && df::global::world->items.all.size() <= MAX_ITEMS, 5);
    auto *item = df::item::find(static_cast<std::int32_t>(id));
    if (!item) return out;
    ws::require(item->id >= 0 && static_cast<std::uint32_t>(item->id) == id, 5);
    if (item->flags.bits.hidden) { out.presence = ws::Presence::Hidden; return out; }
    if (item->pos.x < 0 || item->pos.y < 0 || item->pos.z < 0) return out;
    const ws::Coord p{static_cast<std::uint32_t>(item->pos.x), static_cast<std::uint32_t>(item->pos.y),
        static_cast<std::uint32_t>(item->pos.z)};
    ws::require(p.x < dimensions[0] && p.y < dimensions[1] && p.z < dimensions[2], 5);
    const auto ground = tile(p, all, all_zones);
    if (ground.presence != ws::Presence::Visible) { out.presence = ground.presence; return out; }
    // This second flag word has no field in the frozen capture; refuse it
    // rather than folding fields into indistinguishable witnesses.
    ws::require(item->flags2.whole == 0 && item->general_refs.size() <= MAX_REFS
        && item->specific_refs.size() <= MAX_REFS - item->general_refs.size(), 5);
    out.presence = ws::Presence::Visible; out.pos = p; out.ground = ground;
    const auto type = item->getType(); ws::require(static_cast<int>(type) >= 0 && is_valid_enum_item(type), 5);
    out.log = type == df::item_type::WOOD; out.native_type = static_cast<std::uint32_t>(type);
    out.subtype = item->getSubtype(); out.material = item->getMaterial(); out.material_index = item->getMaterialIndex();
    out.wood = out.log && out.material >= 0 && plant_wood(item);
    const auto quality = item->getQuality(), wear = item->getWear();
    ws::require(quality >= 0 && wear >= 0, 5); out.quality = quality; out.wear = wear;
    auto flags = item->flags; out.on_ground = flags.bits.on_ground; out.in_job = flags.bits.in_job;
    flags.bits.on_ground = false; flags.bits.in_job = false; out.other_flags = flags.whole;
    for (const auto *ref : item->general_refs) { ws::require(ref, 5); ++out.other_refs; }
    for (const auto *ref : item->specific_refs) {
        ws::require(ref, 5);
        if (ref->type == df::specific_ref_type::JOB) {
            ws::require(ref->data.job && ref->data.job->id >= 0 && out.jobs.size() < 8, 5);
            out.jobs.push_back(static_cast<std::uint32_t>(ref->data.job->id));
        } else ++out.other_refs;
    }
    std::sort(out.jobs.begin(), out.jobs.end()); (void)out.encode(); return out;
}
// Complete bounded census of living active citizens (insane included, with the
// fact recorded). More than MAX_WORKERS yields an explicit incomplete census.
std::vector<ws::Worker> census(const std::array<std::uint32_t, 3> &dimensions, bool &complete,
    const std::optional<df::coord> &approach) {
    const auto &active = df::global::world->units.active;
    ws::require(active.size() <= MAX_UNITS, 5);
    std::vector<ws::Worker> out; std::set<std::int32_t> ids; complete = true;
    for (auto *unit : active) {
        ws::require(unit && unit->id >= 0 && unit->id < INT32_MAX && ids.insert(unit->id).second, 5);
        if (!Units::isCitizen(unit, true) || !Units::isAlive(unit) || !Units::isActive(unit)) continue;
        if (out.size() == ws::MAX_WORKERS) { complete = false; continue; }
        ws::require(unit->pos.x >= 0 && unit->pos.y >= 0 && unit->pos.z >= 0
            && static_cast<std::uint32_t>(unit->pos.x) < dimensions[0]
            && static_cast<std::uint32_t>(unit->pos.y) < dimensions[1]
            && static_cast<std::uint32_t>(unit->pos.z) < dimensions[2], 5);
        ws::Worker w; w.id = static_cast<std::uint32_t>(unit->id);
        w.pos = {static_cast<std::uint32_t>(unit->pos.x), static_cast<std::uint32_t>(unit->pos.y),
            static_cast<std::uint32_t>(unit->pos.z)};
        const auto *block = Maps::getTileBlock(unit->pos.x, unit->pos.y, unit->pos.z);
        w.visible = block && !block->designation[unit->pos.x & 15][unit->pos.y & 15].bits.hidden;
        w.adult = Units::isAdult(unit); w.sane = Units::isSane(unit);
        w.labor = unit->status.labors[df::unit_labor::CARPENTER];
        w.labor_valid = Units::isValidLabor(unit, df::unit_labor::CARPENTER);
        w.job_available = Units::isJobAvailable(unit, false);
        w.reachable = w.visible && approach && Maps::canWalkBetween(unit->pos, *approach);
        out.push_back(w);
    }
    if (!complete) out.clear();
    std::sort(out.begin(), out.end(), [](const ws::Worker &a, const ws::Worker &b) { return a.id < b.id; });
    return out;
}
ws::Capture capture(const ws::Selection &selected) {
    (void)selected.encode(); require_fortress();
    const auto clock = clock_identity();
    ws::require(selected.center.x + 1 < clock.dimensions[0] && selected.center.y + 1 < clock.dimensions[1]
        && selected.center.z < clock.dimensions[2], 4);
    ws::require(df::global::job_next_id && *df::global::job_next_id >= 0, 5);
    const auto &all = buildings(); const auto &all_zones = zones(all); ws::Capture out;
    out.selection = selected; out.dimensions = clock.dimensions; out.tick = clock.tick; out.site = clock.site;
    out.folder = clock.folder; out.paused = World::ReadPauseState();
    out.next_building = static_cast<std::uint32_t>(*df::global::building_next_id);
    out.next_job = static_cast<std::uint32_t>(*df::global::job_next_id);
    out.building_count = static_cast<std::uint32_t>(all.size());
    out.zone_count = static_cast<std::uint32_t>(all_zones.size());
    std::size_t index = 0;
    for (auto ty = selected.center.y - 1; ty <= selected.center.y + 1; ++ty)
    for (auto tx = selected.center.x - 1; tx <= selected.center.x + 1; ++tx)
        out.tiles[index++] = tile({tx, ty, selected.center.z}, all, all_zones);
    // Never ask helper APIs to inspect hidden or missing terrain. The exact
    // footprint tags remain visible evidence of why preparation is ineligible.
    bool visible = true;
    for (const auto &t : out.tiles) visible = visible && t.presence == ws::Presence::Visible;
    std::optional<df::coord> approach;
    if (visible) {
        const auto corner = native_pos(selected.corner()); const df::coord2d size(3, 3);
        bool clear = true;
        for (const auto &t : out.tiles) clear = clear && !t.building && !t.zone && !t.pile && !t.smooth;
        out.free_footprint = clear && Buildings::checkFreeTiles(corner, size, nullptr, false, false, false, false);
        out.supported = Buildings::hasSupport(corner, size);
        approach = native_pos(selected.center);
    }
    out.item = item_capture(selected.item, out.dimensions, all, all_zones);
    if (approach && out.item.presence == ws::Presence::Visible)
        out.item.reachable = Maps::canWalkBetween(native_pos(out.item.pos), *approach);
    out.workers = census(out.dimensions, out.census_complete, approach);
    return out;
}
void place(const ws::Capture &before, const std::string &credential) {
    require_fortress(); ws::require(enabled("DFMCP_WORKSHOP_ALLOW_BUILD"), 1);
    ws::require(df::global::plotinfo != nullptr, 5);
    const auto center = before.selection.center;
    auto building = std::unique_ptr<df::building>(Buildings::allocInstance(native_pos(before.selection.corner()),
        df::building_type::Workshop, static_cast<int>(df::workshop_type::Carpenters), -1));
    ws::require(building && building->id == -1 && building->getType() == df::building_type::Workshop, 5);
    const auto *workshop = virtual_cast<df::building_workshopst>(building.get());
    ws::require(workshop && workshop->type == df::workshop_type::Carpenters && workshop->custom_type == -1, 5);
    ws::require(Buildings::setSize(building.get(), df::coord2d(3, 3), 0), 5);
    ws::require(building->x1 == static_cast<std::int32_t>(center.x - 1) && building->x2 == static_cast<std::int32_t>(center.x + 1)
        && building->y1 == static_cast<std::int32_t>(center.y - 1) && building->y2 == static_cast<std::int32_t>(center.y + 1)
        && building->z == static_cast<std::int32_t>(center.z) && building->centerx == static_cast<std::int32_t>(center.x)
        && building->centery == static_cast<std::int32_t>(center.y), 5);
    std::vector<df::item *> selected_items;
    selected_items.reserve(1);
    auto *selected = df::item::find(static_cast<std::int32_t>(before.selection.item));
    ws::require(selected, 6); selected_items.push_back(selected);
    // Allocation-only helpers can fail. Revalidate everything again after them
    // and immediately before handing ownership to the effectful API.
    auto current = capture(before.selection); current.generation = before.generation; current.sequence = before.sequence;
    ws::require(engine.generation() == before.generation && engine.sequence() == before.sequence + 1
        && current.encode() == before.encode() && current.eligible(), 6);
    ws::require(development_profile() && enabled("DFMCP_WORKSHOP_ALLOW_BUILD"), 1);
    // Credential rotation or removal revokes this dispatch even after entry
    // authentication. The secret never leaves this suspended call.
    authorize_credential(credential);
    // constructWithItems may throw or return false after any registry, job,
    // item or occupancy link. Ownership is relinquished at the call boundary;
    // only independent readback can publish Placed.
    auto *native = building.release();
    (void)Buildings::constructWithItems(native, std::move(selected_items));
}
bool construct_job(const df::job *job) { return job && job->job_type == df::job_type::ConstructBuilding; }
ws::Insertion verify(const ws::Capture &before) {
    require_fortress(); const auto &all = buildings(); (void)zones(all); df::building *building = nullptr;
    for (auto *candidate : all) if (static_cast<std::uint32_t>(candidate->id) == before.next_building) building = candidate;
    const auto center = before.selection.center;
    ws::require(building && building->getType() == df::building_type::Workshop, 5);
    auto *workshop = virtual_cast<df::building_workshopst>(building);
    ws::require(workshop && workshop->type == df::workshop_type::Carpenters && workshop->custom_type == -1
        && building->x1 == static_cast<std::int32_t>(center.x - 1) && building->x2 == static_cast<std::int32_t>(center.x + 1)
        && building->y1 == static_cast<std::int32_t>(center.y - 1) && building->y2 == static_cast<std::int32_t>(center.y + 1)
        && building->z == static_cast<std::int32_t>(center.z) && building->centerx == static_cast<std::int32_t>(center.x)
        && building->centery == static_cast<std::int32_t>(center.y)
        && building->mat_type == before.item.material && building->mat_index == before.item.material_index
        && building->jobs.size() == 1 && building->general_refs.empty() && building->specific_refs.empty()
        && building->relations.empty() && !building->flags.bits.exists && workshop->contained_items.empty(), 5);
    ws::Insertion proof;
    proof.setting_occupancy = building->isSettingOccupancy();
    std::size_t index = 0;
    for (auto ty = center.y - 1; ty <= center.y + 1; ++ty)
    for (auto tx = center.x - 1; tx <= center.x + 1; ++tx) {
        const auto *block = Maps::getTileBlock(static_cast<std::int32_t>(tx), static_cast<std::int32_t>(ty),
            static_cast<std::int32_t>(center.z));
        ws::require(block, 5);
        if (block->occupancy[tx & 15][ty & 15].bits.building == df::tile_building_occ::Planned)
            proof.planned_tiles = static_cast<std::uint16_t>(proof.planned_tiles | (1u << index));
        ++index;
    }
    auto *job = building->jobs[0]; ws::require(job && job->id >= 0
        && static_cast<std::uint32_t>(job->id) == before.next_job, 5);
    std::set<std::int32_t> job_ids; bool found = false;
    auto *previous = &df::global::world->jobs.list;
    for (auto *link = df::global::world->jobs.list.next; link; link = link->next) {
        ws::require(job_ids.size() < MAX_JOBS && link->item && link->item->id >= 0
            && link->prev == previous && link->item->id < *df::global::job_next_id
            && job_ids.insert(link->item->id).second && link->item->list_link == link, 5);
        if (link->item == job) { ws::require(job->list_link == link, 5); found = true; }
        previous = link;
    }
    ws::require(found && construct_job(job)
        && job->pos.x == building->centerx && job->pos.y == building->centery && job->pos.z == building->z
        && job->mat_type == building->mat_type && job->mat_index == building->mat_index
        && job->flags.whole == 0 && job->job_items.elements.empty() && job->items.size() == 1 && job->specific_refs.empty()
        && job->general_refs.size() == 1 && job->general_refs[0]
        && job->general_refs[0]->getType() == df::general_ref_type::BUILDING_HOLDER
        && job->general_refs[0]->getBuilding() == building && Job::getHolder(job) == building, 5);
    auto *item = df::item::find(static_cast<std::int32_t>(before.selection.item));
    const auto *attachment = job->items[0];
    ws::require(item && attachment && attachment->item == item
        && attachment->role == df::job_role_type::Hauled && attachment->job_item_idx == -1
        && attachment->flags.whole == 0 && item->general_refs.empty() && item->specific_refs.size() == 1
        && item->specific_refs[0] && item->specific_refs[0]->type == df::specific_ref_type::JOB
        && item->specific_refs[0]->data.job == job && item->flags.bits.in_job, 5);
    const auto stage = building->getBuildStage(), max_stage = building->getMaxBuildStage();
    ws::require(stage == 0 && max_stage >= 1 && max_stage <= 32, 5);
    proof.building = before.next_building; proof.job = before.next_job; proof.item = before.selection.item;
    proof.kind = before.selection.kind;
    proof.x1 = center.x - 1; proof.x2 = center.x + 1; proof.y1 = center.y - 1; proof.y2 = center.y + 1;
    proof.z = center.z; proof.center_x = center.x; proof.center_y = center.y;
    proof.material = building->mat_type; proof.material_index = building->mat_index;
    proof.stage = static_cast<std::uint32_t>(stage); proof.max_stage = static_cast<std::uint32_t>(max_stage);
    proof.building_flags = building->flags.whole; proof.has_design = workshop->design != nullptr;
    proof.linked = true; proof.construct_job = true; proof.exact_item_link = true; proof.suspended = job->flags.bits.suspend;
    return proof;
}
// Fresh evidence for one original building ID. It does not require the
// original log to remain available or any worker to remain idle.
ws::Progress progress(std::uint32_t id, std::uint32_t item) {
    require_fortress(); const auto clock = clock_identity(); const auto &all = buildings(); (void)zones(all);
    ws::Progress out; out.tick = clock.tick; out.site = clock.site; out.dimensions = clock.dimensions;
    out.folder = clock.folder; out.next_building = static_cast<std::uint32_t>(*df::global::building_next_id);
    out.building = id; out.item = item;
    df::building *building = nullptr;
    for (auto *candidate : all) if (static_cast<std::uint32_t>(candidate->id) == id) building = candidate;
    if (!building) return out;
    out.present = true;
    auto *workshop = virtual_cast<df::building_workshopst>(building);
    out.carpenters = building->getType() == df::building_type::Workshop && workshop
        && workshop->type == df::workshop_type::Carpenters && workshop->custom_type == -1;
    out.subtype = building->getSubtype(); out.custom = building->getCustomType();
    out.x1 = static_cast<std::uint32_t>(building->x1); out.y1 = static_cast<std::uint32_t>(building->y1);
    out.x2 = static_cast<std::uint32_t>(building->x2); out.y2 = static_cast<std::uint32_t>(building->y2);
    out.z = static_cast<std::uint32_t>(building->z);
    ws::require(building->centerx >= 0 && building->centery >= 0, 5);
    out.center_x = static_cast<std::uint32_t>(building->centerx); out.center_y = static_cast<std::uint32_t>(building->centery);
    out.material = building->mat_type; out.material_index = building->mat_index;
    const auto stage = building->getBuildStage(), max_stage = building->getMaxBuildStage();
    ws::require(stage >= 0 && max_stage >= 1 && max_stage <= 32 && stage <= max_stage, 5);
    out.stage = static_cast<std::uint32_t>(stage); out.max_stage = static_cast<std::uint32_t>(max_stage);
    out.exists = building->flags.bits.exists;
    ws::require(building->jobs.size() <= MAX_BUILDING_JOBS, 5);
    for (const auto *job : building->jobs) {
        ws::require(job, 5); ++out.jobs; if (construct_job(job)) ++out.construct_jobs;
    }
    if (workshop) {
        ws::require(workshop->contained_items.size() <= MAX_CONTAINED, 5);
        for (const auto *contained : workshop->contained_items) {
            ws::require(contained && contained->item && contained->item->id >= 0, 5); ++out.contained;
            if (static_cast<std::uint32_t>(contained->item->id) == item) out.item_contained = true;
        }
    }
    return out;
}
unsigned shape(const wire::Request *in) {
    return (in->has_item_id() ? 1 : 0) | (in->has_x() ? 2 : 0) | (in->has_y() ? 4 : 0)
        | (in->has_z() ? 8 : 0) | (in->has_idempotency_key() ? 16 : 0) | (in->has_expected_witness() ? 32 : 0)
        | (in->has_plan_digest() ? 64 : 0) | (in->has_prepare_token() ? 128 : 0) | (in->has_building_id() ? 256 : 0);
}
constexpr unsigned READ = 15, PREPARE = 127, COMMIT = 208, QUERY = 80, PROGRESS = 257;
ws::Selection selection(const wire::Request *in) {
    ws::Selection out{ws::Kind::Carpenters, in->item_id(), {in->x(), in->y(), in->z()}};
    (void)out.encode(); return out;
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out,
    unsigned expected_shape, bool building, Body body) {
    try {
        authorize(in, out); ws::require(shape(in) == expected_shape);
        if (building) ws::require(enabled("DFMCP_WORKSHOP_ALLOW_BUILD"), 1);
        manifest(out); body(); out->set_unresolved(engine.unresolved());
        out->set_retained_records(static_cast<std::uint32_t>(engine.size()));
        out->set_failure_code(0); out->set_accepted(true); ws::require(static_cast<std::size_t>(out->ByteSize()) <= MAX_REPLY_BYTES, 5);
        return CR_OK;
    } catch (const ws::Failure &e) {
        try { empty_reply(in, out, e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in, out, 5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, 0, false, [] {});
}
command_result ReadWorkshop(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, READ, false, [&] { out->set_observation(engine.inspect(selection(in), capture).encode()); });
}
command_result PrepareWorkshop(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, PREPARE, true, [&] {
        const bool replay = engine.query(in->idempotency_key(), in->plan_digest()) != nullptr;
        const auto &record = engine.prepare(in->idempotency_key(), selection(in), in->expected_witness(),
            in->plan_digest(), monotonic_ms(), capture);
        out->set_effect_record(record.encode()); out->set_replayed(replay);
    });
}
command_result CommitWorkshop(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, COMMIT, true, [&] {
        out->set_effect_record(engine.commit(in->idempotency_key(), in->plan_digest(), in->prepare_token(),
            monotonic_ms(), capture, [&](const ws::Capture &before) {
                place(before, in->bearer_token());
            }, verify).encode());
    });
}
command_result QueryWorkshop(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, QUERY, false, [&] {
        // Read-only recovery can finish the original proof; no writer is reachable.
        if (const auto *record = engine.reconcile(in->idempotency_key(), in->plan_digest(),
            capture, verify)) out->set_effect_record(record->encode());
    });
}
command_result CancelWorkshop(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, COMMIT, false, [&] {
        out->set_effect_record(engine.cancel(in->idempotency_key(), in->plan_digest(), in->prepare_token()).encode());
    });
}
command_result ReadWorkshopProgress(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, PROGRESS, false, [&] {
        out->set_observation(engine.progress(in->building_id(), in->item_id(), progress).encode());
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
// Zero flags: DFHack's suspended dispatch owns every read/prepare/commit.
// No native pointer, scheduled work, timer or effect is retained across calls.
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ReadWorkshop", ReadWorkshop, 0);
    service->addFunction("PrepareWorkshop", PrepareWorkshop, 0); service->addFunction("CommitWorkshop", CommitWorkshop, 0);
    service->addFunction("QueryWorkshop", QueryWorkshop, 0); service->addFunction("CancelWorkshop", CancelWorkshop, 0);
    service->addFunction("ReadWorkshopProgress", ReadWorkshopProgress, 0);
    return service;
}
