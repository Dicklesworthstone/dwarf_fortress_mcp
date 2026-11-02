#pragma once
// Explicit API doubles for compiling the unmodified workshop/1.20 plugin
// translation unit. These layouts are NOT DFHack ABI, generated protobuf
// serialization or game behavior; they emulate only the documented SDK calls.
#include <array>
#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <set>
#include <stdexcept>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

namespace mock {
inline bool loaded = true, mapped = true, fortress = true, paused = true;
inline bool initialized = true, unknown = false, reply_failure = false, oversized_reply = false, oversized_request = false;
inline bool free_tiles = true, supported = true, allocation_failure = false, resize_failure = false;
inline bool setting_occupancy = true, needs_design = true, invalid_dfhack_version = false;
inline unsigned game_access = 0, writer_calls = 0, allocator_calls = 0, walk_calls = 0, helper_calls = 0;
inline std::int64_t year = 0;
inline std::uint32_t tick = 12345;
inline std::int32_t site = 1, size_x = 64, size_y = 64, size_z = 8;
inline std::string folder = "region1", df_version = "test-df", dfhack_version = "test-dfhack";
inline std::int32_t next_building = 70, next_job = 90;
inline std::int32_t next_item = 1000;
inline bool helper_arguments_safe = true;
// Equal nonzero walk groups model Maps::canWalkBetween only; no route or assignment.
inline std::map<std::tuple<int, int, int>, int> walk_groups;
enum class WriteFault { None, FalseBefore, ThrowBefore, BuildingOnly, JobOnly, FalseAfter, ThrowAfter };
inline WriteFault write_fault = WriteFault::None;
}

namespace df {
struct building;
struct item;
struct job;
struct job_list_link;
struct plant;
struct coord {
    std::int32_t x = 0, y = 0, z = 0;
    coord() = default;
    coord(int xx, int yy, int zz) : x(xx), y(yy), z(zz) {}
    bool operator==(const coord &other) const { return x == other.x && y == other.y && z == other.z; }
    bool operator!=(const coord &other) const { return !(*this == other); }
};
struct coord2d {
    std::int32_t x = 0, y = 0;
    coord2d() = default;
    coord2d(int xx, int yy) : x(xx), y(yy) {}
};
enum class tiletype { Floor = 1, Wall = 2, Empty = 3, Ramp = 4, RampTop = 5,
    StairUp = 6, StairDown = 7, StairUpDown = 8, Unclassified = 9, TreeWall = 10, Bad = 99 };
enum class tiletype_material { STONE, TREE };
enum class tiletype_shape { EMPTY, WALL, FLOOR, RAMP, RAMP_TOP, STAIR_UP, STAIR_DOWN, STAIR_UPDOWN, NONE };
enum class tile_dig_designation : std::uint32_t { No = 0, Default = 1, Channel = 2 };
enum class tile_building_occ : std::uint32_t { None = 0, Planned = 1, Passable = 2, Impassable = 3 };
enum class item_type : std::int32_t { NONE = -1, BED = 1, BOULDER = 4, WOOD = 5, BAR = 6, WEAPON = 7, INVALID = 99 };
enum class building_type : std::int32_t { NONE = -1, Bed = 1, Stockpile = 4, Civzone = 5, Workshop = 6, Wagon = 7 };
enum class workshop_type : std::int32_t { Carpenters = 0, Farmers = 1, Masons = 2, Custom = 23 };
enum class job_type : std::int32_t { ConstructBuilding = 1, DestroyBuilding = 2, FellTree = 3, GatherPlants = 4 };
enum class job_skill { AXE, SWORD };
struct itemdef_weaponst { job_skill skill_melee = job_skill::AXE; static itemdef_weaponst *find(int subtype) {
    static itemdef_weaponst axe; return subtype == 0 ? &axe : nullptr;
} };
enum class job_role_type : std::int32_t { Hauled = 0, Reagent = 1 };
enum class specific_ref_type : std::int32_t { NONE = 0, JOB = 1, UNIT = 2 };
enum class general_ref_type : std::int32_t { NONE = 0, BUILDING_HOLDER = 1, UNIT_WORKER = 2 };
enum class material_flags : std::int32_t { WOOD = 23, STONE = 24 };
// Unscoped like the generated enum, so labor arrays index without casts.
enum unit_labor : std::int16_t { NONE_LABOR = -1, MINE = 0, CUTWOOD = 10, CARPENTER = 11, MASON = 12, LABOR_COUNT = 94 };
union tile_designation {
    std::uint32_t whole;
    struct { tile_dig_designation dig : 3; std::uint32_t hidden : 1, flow_size : 3, pile : 1, smooth : 2, other : 22; } bits;
    tile_designation() : whole(0) {}
};
union tile_occupancy {
    std::uint32_t whole;
    struct { tile_building_occ building : 3; std::uint32_t unit : 1, unit_grounded : 1, item : 1, other : 26; } bits;
    tile_occupancy() : whole(0) {}
};
union item_flags {
    std::uint32_t whole;
    struct { std::uint32_t on_ground : 1, in_job : 1, hidden : 1, removed : 1,
        forbid : 1, dump : 1, rotten : 1, trader : 1, in_inventory : 1, in_building : 1,
        other : 22; } bits;
    item_flags() : whole(0) {}
};
union building_flags {
    std::uint32_t whole;
    struct { std::uint32_t exists : 1, site_blocked : 1, room_collision : 1, other : 29; } bits;
    building_flags() : whole(0) {}
};
struct raw_flags { std::uint32_t whole = 0; };
struct specific_ref {
    df::specific_ref_type type = specific_ref_type::NONE;
    struct { df::job *job = nullptr; } data;
};
struct general_ref {
    df::general_ref_type type = general_ref_type::NONE;
    df::building *holder = nullptr;
    virtual ~general_ref() = default;
    virtual general_ref_type getType() const { return type; }
    virtual building *getBuilding() const { return holder; }
};
struct general_ref_building_holderst : general_ref {
    std::int32_t building_id = -1;
    general_ref_building_holderst() { type = general_ref_type::BUILDING_HOLDER; }
};
struct item {
    std::int32_t id = 42;
    coord pos{10, 11, 2};
    item_flags flags;
    raw_flags flags2;
    std::vector<general_ref *> general_refs;
    std::vector<specific_ref *> specific_refs;
    item_type type = item_type::WOOD;
    std::int32_t subtype = -1, material = 420, material_index = 163, quality = 0, wear = 0;
    virtual ~item() = default;
    item() { flags.bits.on_ground = true; }
    item_type getType() { return type; }
    int getSubtype() { return subtype; }
    std::int16_t getMaterial() { return static_cast<std::int16_t>(material); }
    int getMaterialIndex() { return material_index; }
    int getQuality() { return quality; }
    int getWear() { return wear; }
    static item *find(int id);
};
struct material {
    struct Flags {
        std::set<material_flags> values;
        bool is_set(material_flags flag) const { return values.count(flag) != 0; }
    } flags;
};
struct job_item_ref {
    df::item *item = nullptr;
    job_role_type role = job_role_type::Hauled;
    std::int32_t job_item_idx = -1;
    raw_flags flags;
};
union job_flags {
    std::uint32_t whole;
    struct { std::uint32_t suspend : 1, repeat : 1, other : 30; } bits;
    job_flags() : whole(0) {}
};
struct job {
    std::int32_t id = -1;
    coord pos;
    df::job_type job_type = df::job_type::ConstructBuilding;
    job_flags flags;
    std::vector<general_ref *> general_refs;
    std::vector<job_item_ref *> items;
    std::vector<specific_ref *> specific_refs;
    struct { std::vector<void *> elements; } job_items;
    job_list_link *list_link = nullptr;
    std::int32_t mat_type = -1, mat_index = -1;
};
struct job_list_link { job *item = nullptr; job_list_link *next = nullptr, *prev = nullptr; };
struct unit {
    std::int32_t id = 1;
    coord pos{20, 20, 2};
    bool citizen = true, alive = true, active = true, sane = true, adult = true;
    bool civ_permits_carpentry = true, has_current_job = false, caged = false;
    struct { std::array<bool, LABOR_COUNT> labors{}; } status;
    virtual ~unit() = default;
};
inline std::set<building *> live_buildings;
struct building_design { std::int32_t builder1 = -1; };
struct buildingitemst { df::item *item = nullptr; std::int16_t use_mode = 0; };
struct building {
    std::int32_t id = -1, x1 = 0, y1 = 0, x2 = 0, y2 = 0, z = 0, centerx = 0, centery = 0;
    std::int32_t mat_type = -1, mat_index = -1, stage = 0, maximum_stage = 3;
    building_type type = building_type::Bed;
    std::vector<job *> jobs;
    std::vector<building *> relations;
    std::vector<general_ref *> general_refs;
    std::vector<specific_ref *> specific_refs;
    building_flags flags;
    struct { std::int32_t x = 0, y = 0, width = 0, height = 0; } room;
    building() { live_buildings.insert(this); }
    virtual ~building() { live_buildings.erase(this); }
    virtual building_type getType() { return type; }
    virtual std::int32_t getSubtype() { return -1; }
    virtual std::int32_t getCustomType() { return -1; }
    virtual std::int32_t getBuildStage() { return stage; }
    virtual std::int32_t getMaxBuildStage() { return maximum_stage; }
    virtual bool isSettingOccupancy() { return true; }
    virtual bool needsDesign() { return false; }
};
struct building_actual : building {
    std::vector<buildingitemst *> contained_items;
    building_design *design = nullptr;
    ~building_actual() override { delete design; }
};
struct building_wagonst : building_actual {building_wagonst() {type=building_type::Wagon;}};
struct building_workshopst : building_actual {
    workshop_type type = workshop_type::Carpenters;
    std::int32_t custom_type = -1;
    building_workshopst() { building::type = building_type::Workshop; }
    std::int32_t getSubtype() override { return static_cast<std::int32_t>(type); }
    std::int32_t getCustomType() override { return custom_type; }
    bool isSettingOccupancy() override { return mock::setting_occupancy; }
    bool needsDesign() override { return mock::needs_design; }
};
struct building_civzonest : building_actual {
    building_civzonest() { type = building_type::Civzone; }
    bool isSettingOccupancy() override { return false; }
};
struct map_block {
    union BlockFlags { std::uint32_t whole; struct { std::uint32_t designated:1,other:31; } bits; BlockFlags():whole(0){} } flags;
    std::array<std::array<std::uint16_t,16>,16> temperature_1{},temperature_2{};
    std::array<std::array<tile_designation, 16>, 16> designation{};
    std::array<std::array<tile_occupancy, 16>, 16> occupancy{};
    std::array<std::array<df::tiletype, 16>, 16> tiletype{};
    map_block() { for (auto &column : tiletype) column.fill(df::tiletype::Floor);
        for(auto &c:temperature_1) {c.fill(10000);}
        for(auto &c:temperature_2) {c.fill(10000);} }
};
struct world {
    struct { std::vector<building *> all; struct { std::vector<building_civzonest *> ANY_ZONE; } other; } buildings;
    struct { std::vector<item *> all; } items;
    struct { std::vector<unit *> active; } units;
    struct { job_list_link list; } jobs;
    struct { std::vector<plant *> all; } plants;
};
namespace global {
inline df::world *world = nullptr;
inline void *plotinfo = nullptr;
inline std::int32_t *building_next_id = &mock::next_building, *job_next_id = &mock::next_job;
inline std::int32_t *item_next_id = &mock::next_item;
}
inline item *item::find(int id) {
    ++mock::game_access;
    if (global::world) for (auto *value : global::world->items.all) if (value && value->id == id) return value;
    return nullptr;
}
}

namespace mock {
inline std::map<std::tuple<int, int, int>, std::unique_ptr<df::map_block>> blocks;
inline std::vector<std::unique_ptr<df::item>> items;
inline std::vector<std::unique_ptr<df::unit>> units;
inline std::vector<std::unique_ptr<df::job>> jobs;
inline std::vector<std::unique_ptr<df::job_list_link>> links;
inline std::vector<std::unique_ptr<df::general_ref>> general_refs;
inline std::vector<std::unique_ptr<df::specific_ref>> specific_refs;
inline std::vector<std::unique_ptr<df::job_item_ref>> item_refs;
inline std::vector<std::unique_ptr<df::buildingitemst>> contained;
inline std::map<std::pair<int, int>, df::material> materials;
inline std::function<void(df::building *, df::job *, df::item *)> after_construct;
inline std::function<void()> after_allocation;
inline std::function<void(df::job *)> during_verify;
}

namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED,
    SC_PAUSED, SC_UNPAUSED, SC_VIEWSCREEN_CHANGED, SC_OTHER };
template<class T, class P> T *virtual_cast(P *ptr) { return dynamic_cast<T *>(ptr); }
struct VersionInfo { std::string getVersion() const { return mock::df_version; } };
struct Core {
    std::shared_ptr<VersionInfo> vinfo = std::make_shared<VersionInfo>();
    static Core &getInstance() { static Core value; return value; }
    bool isWorldLoaded() const { ++mock::game_access; return mock::loaded; }
    bool isMapLoaded() const { ++mock::game_access; return mock::mapped; }
};
namespace Version {
inline const char *dfhack_version() { return mock::invalid_dfhack_version ? nullptr : mock::dfhack_version.c_str(); }
}
namespace World {
inline bool isFortressMode() { ++mock::game_access; return mock::fortress; }
inline bool ReadPauseState() { ++mock::game_access; return mock::paused; }
inline std::int64_t ReadCurrentYear() { ++mock::game_access; return mock::year; }
inline std::uint32_t ReadCurrentTick() { ++mock::game_access; return mock::tick; }
inline std::int32_t GetCurrentSiteId() { ++mock::game_access; return mock::site; }
inline std::string ReadWorldFolder() { ++mock::game_access; return mock::folder; }
}
struct MaterialInfo {
    df::material *material = nullptr;
    bool plant = false;
    MaterialInfo(std::int16_t type, std::int32_t index) {
        const auto it = mock::materials.find({type, index});
        if (it != mock::materials.end()) { material = &it->second; plant = type >= 419 && type < 619; }
    }
    bool isPlant() const { return plant; }
};
inline bool is_valid_enum_item(df::tiletype type) { return type != df::tiletype::Bad && static_cast<int>(type) >= 0; }
inline bool is_valid_enum_item(df::item_type type) { return type != df::item_type::INVALID && static_cast<int>(type) >= 0; }
inline df::tiletype_shape tileShape(df::tiletype type) {
    switch (type) {
        case df::tiletype::Floor: return df::tiletype_shape::FLOOR;
        case df::tiletype::Wall: case df::tiletype::TreeWall: return df::tiletype_shape::WALL;
        case df::tiletype::Empty: return df::tiletype_shape::EMPTY;
        case df::tiletype::Ramp: return df::tiletype_shape::RAMP;
        case df::tiletype::RampTop: return df::tiletype_shape::RAMP_TOP;
        case df::tiletype::StairUp: return df::tiletype_shape::STAIR_UP;
        case df::tiletype::StairDown: return df::tiletype_shape::STAIR_DOWN;
        case df::tiletype::StairUpDown: return df::tiletype_shape::STAIR_UPDOWN;
        default: return df::tiletype_shape::NONE;
    }
}
inline df::tiletype_material tileMaterial(df::tiletype type) {return type==df::tiletype::TreeWall?df::tiletype_material::TREE:df::tiletype_material::STONE;}
namespace Units {
inline bool isCitizen(df::unit *u, bool include_insane = false) { ++mock::game_access; return u->citizen && (include_insane || u->sane); }
inline bool isAlive(df::unit *u) { ++mock::game_access; return u->alive; }
inline bool isActive(df::unit *u) { ++mock::game_access; return u->active; }
inline bool isSane(df::unit *u) { ++mock::game_access; return u->sane; }
inline bool isAdult(df::unit *u) { ++mock::game_access; return u->adult; }
// Inspected SDK semantics: civilization permission only, not the enabled bit.
inline bool isValidLabor(df::unit *u, df::unit_labor labor) {
    ++mock::game_access; return (labor == df::unit_labor::CARPENTER || labor == df::unit_labor::CUTWOOD) && u->civ_permits_carpentry;
}
// Inspected SDK semantics: current job/cage/activity, not scheduler choice.
inline bool isJobAvailable(df::unit *u, bool preserve_social = false) {
    ++mock::game_access; return !preserve_social && !u->has_current_job && !u->caged;
}
}
namespace Maps {
inline bool IsValid() { ++mock::game_access; return mock::mapped; }
inline void getTileSize(int &x, int &y, int &z) {
    ++mock::game_access; x = mock::size_x; y = mock::size_y; z = mock::size_z;
}
inline df::map_block *getTileBlock(int x, int y, int z) {
    ++mock::game_access;
    const auto it = mock::blocks.find(std::make_tuple(x / 16, y / 16, z));
    return it == mock::blocks.end() ? nullptr : it->second.get();
}
inline df::map_block *getTileBlock(df::coord p) {return getTileBlock(p.x,p.y,p.z);}
inline bool canWalkBetween(df::coord a, df::coord b) {
    ++mock::game_access; ++mock::walk_calls;
    const auto first = mock::walk_groups.find({a.x, a.y, a.z}), second = mock::walk_groups.find({b.x, b.y, b.z});
    return first != mock::walk_groups.end() && second != mock::walk_groups.end()
        && first->second != 0 && first->second == second->second;
}
}
namespace Job {
inline df::building *getHolder(df::job *job) {
    ++mock::game_access;
    if (mock::during_verify) mock::during_verify(job);
    for (auto *ref : job->general_refs)
        if (ref && ref->getType() == df::general_ref_type::BUILDING_HOLDER) return ref->getBuilding();
    return nullptr;
}
}
namespace Buildings {
inline bool checkFreeTiles(df::coord pos, df::coord2d size, df::building *building = nullptr,
    bool create_ext = false, bool allow_occupied = false, bool allow_wall = false, bool allow_flow = false) {
    ++mock::game_access; ++mock::helper_calls;
    mock::helper_arguments_safe &= size.x == 3 && size.y == 3 && !building
        && !create_ext && !allow_occupied && !allow_wall && !allow_flow;
    for (int dx = 0; dx < size.x; ++dx) for (int dy = 0; dy < size.y; ++dy) {
        auto *block = Maps::getTileBlock(pos.x + dx, pos.y + dy, pos.z);
        if (!block || block->occupancy[(pos.x + dx) & 15][(pos.y + dy) & 15].bits.building != df::tile_building_occ::None) return false;
    }
    return mock::free_tiles;
}
inline bool hasSupport(df::coord, df::coord2d size) {
    ++mock::game_access; ++mock::helper_calls;
    mock::helper_arguments_safe &= size.x == 3 && size.y == 3;
    return mock::supported;
}
// Mirrors the inspected SDK: base fields at pos, subtype through the virtual
// setter, material amount bookkeeping is not modeled.
inline df::building *allocInstance(df::coord pos, df::building_type type, int subtype = -1, int custom = -1) {
    ++mock::game_access; ++mock::allocator_calls;
    if (mock::allocation_failure || type != df::building_type::Workshop) return nullptr;
    auto *building = new df::building_workshopst();
    building->x1 = building->x2 = building->centerx = pos.x;
    building->y1 = building->y2 = building->centery = pos.y; building->z = pos.z;
    if (subtype != -1) building->type = static_cast<df::workshop_type>(subtype);
    if (custom != -1) building->custom_type = custom;
    if (mock::after_allocation) mock::after_allocation();
    return building;
}
inline bool setSize(df::building *building, df::coord2d size, int direction = 0) {
    ++mock::game_access;
    if (mock::resize_failure || direction || building->id != -1) return false;
    // Workshops other than Quern/Millstone/Tool/Siege/Kennels/Custom are fixed 3x3, center 1,1.
    (void)size; building->x2 = building->x1 + 2; building->y2 = building->y1 + 2;
    building->centerx = building->x1 + 1; building->centery = building->y1 + 1;
    return true;
}
inline bool constructWithItems(df::building *building, std::vector<df::item *> items) {
    ++mock::game_access; ++mock::writer_calls;
    if (mock::write_fault == mock::WriteFault::FalseBefore) return false;
    if (mock::write_fault == mock::WriteFault::ThrowBefore) throw std::runtime_error("fault before registration");
    if (items.empty() || items.front()->flags.bits.in_job) return false;
    auto *item = items.front();
    building->id = (*df::global::building_next_id)++;
    df::global::world->buildings.all.push_back(building);
    if (building->isSettingOccupancy())
        for (int x = building->x1; x <= building->x2; ++x) for (int y = building->y1; y <= building->y2; ++y) {
            auto *block = Maps::getTileBlock(x, y, building->z);
            block->occupancy[x & 15][y & 15].bits.building = df::tile_building_occ::Planned;
            block->designation[x & 15][y & 15].bits.dig = df::tile_dig_designation::No;
            block->designation[x & 15][y & 15].bits.pile = false;
        }
    if (mock::write_fault == mock::WriteFault::BuildingOnly) throw std::runtime_error("fault after building registration");
    auto job = std::make_unique<df::job>(); job->id = (*df::global::job_next_id)++;
    job->pos = {building->centerx, building->centery, building->z};
    auto holder = std::make_unique<df::general_ref_building_holderst>();
    holder->holder = building; holder->building_id = building->id;
    job->general_refs.push_back(holder.get()); mock::general_refs.push_back(std::move(holder));
    auto link = std::make_unique<df::job_list_link>(); link->item = job.get();
    link->next = df::global::world->jobs.list.next; link->prev = &df::global::world->jobs.list;
    if (link->next) link->next->prev = link.get();
    df::global::world->jobs.list.next = link.get(); job->list_link = link.get();
    mock::links.push_back(std::move(link)); building->jobs.push_back(job.get());
    auto *native_job = job.get(); mock::jobs.push_back(std::move(job));
    if (mock::write_fault == mock::WriteFault::JobOnly) throw std::runtime_error("fault after job registration");
    for (auto *selected : items) {
        auto ref = std::make_unique<df::job_item_ref>(); ref->item = selected;
        native_job->items.push_back(ref.get()); mock::item_refs.push_back(std::move(ref));
        auto reverse = std::make_unique<df::specific_ref>(); reverse->type = df::specific_ref_type::JOB;
        reverse->data.job = native_job; selected->specific_refs.push_back(reverse.get());
        mock::specific_refs.push_back(std::move(reverse)); selected->flags.bits.in_job = true;
        if (building->mat_type == -1) building->mat_type = selected->material;
        if (building->mat_index == -1) building->mat_index = selected->material_index;
    }
    native_job->mat_type = building->mat_type; native_job->mat_index = building->mat_index;
    if (building->needsDesign()) static_cast<df::building_actual *>(building)->design = new df::building_design();
    if (mock::after_construct) mock::after_construct(building, native_job, item);
    if (mock::write_fault == mock::WriteFault::ThrowAfter) throw std::runtime_error("fault after complete registration");
    return mock::write_fault != mock::WriteFault::FalseAfter;
}
}
struct RPCService {
    std::vector<std::pair<std::string, int>> methods;
    template<class F> void addFunction(const char *name, F, int flags) { methods.emplace_back(name, flags); }
};
}
#define DFHACK_PLUGIN(name) static_assert(sizeof(name) > 1, "plugin name")
#define DFhackCExport extern "C"

namespace dfmcp::workshop::v1_20 {
struct Request {
    unsigned mask = 0;
    std::string bearer = std::string(32, 's'), nonce = std::string(16, 'n');
    unsigned major = 1, minor = 20;
    std::uint32_t item = 42, xx = 15, yy = 15, zz = 2, building = 70;
    std::string key = "workshop-001", witness, plan, token;
    struct Fields { bool empty() const { return !mock::unknown; } };
    struct Reflection { Fields GetUnknownFields(const Request &) const { return {}; } };
    const Reflection *GetReflection() const { static Reflection value; return &value; }
    bool IsInitialized() const { return mock::initialized; }
    std::size_t ByteSize() const { return mock::oversized_request ? 2049 : 64 + bearer.size() + nonce.size() + key.size() + witness.size() + plan.size() + token.size(); }
    const std::string &bearer_token() const { return bearer; }
    const std::string &client_nonce() const { return nonce; }
    unsigned protocol_major() const { return major; }
    unsigned protocol_minor() const { return minor; }
#define WS_FIELD(type, name, member, bit) bool has_##name() const { return (mask & (bit)) != 0; } type name() const { return member; }
    WS_FIELD(std::uint32_t, item_id, item, 1)
    WS_FIELD(std::uint32_t, x, xx, 2)
    WS_FIELD(std::uint32_t, y, yy, 4)
    WS_FIELD(std::uint32_t, z, zz, 8)
    WS_FIELD(const std::string &, idempotency_key, key, 16)
    WS_FIELD(const std::string &, expected_witness, witness, 32)
    WS_FIELD(const std::string &, plan_digest, plan, 64)
    WS_FIELD(const std::string &, prepare_token, token, 128)
    WS_FIELD(std::uint32_t, building_id, building, 256)
#undef WS_FIELD
};
struct Reply {
    bool accepted = false, replayed = false, unresolved = false;
    unsigned code = 0, major = 0, minor = 0, records = 0, mask = 0;
    std::uint64_t generation = 0;
    std::string nonce, df_version, dfhack_version, observation, effect;
    void Clear() { *this = Reply{}; }
    void set_accepted(bool value) { accepted = value; }
    void set_failure_code(unsigned value) { code = value; }
    void set_client_nonce(const std::string &value) { nonce = value; }
    void set_protocol_major(unsigned value) { major = value; }
    void set_protocol_minor(unsigned value) { minor = value; }
    void set_bridge_generation(std::uint64_t value) { generation = value; }
    void set_df_version(const std::string &value) { df_version = value; }
    void set_dfhack_version(const std::string &value) { dfhack_version = value; }
    void set_observation(const std::string &value) { mask |= 1; observation = value; }
    void set_effect_record(const std::string &value) {
        if (mock::reply_failure) { mock::reply_failure = false; throw std::bad_alloc(); }
        mask |= 2; effect = value;
    }
    void set_replayed(bool value) { mask |= 4; replayed = value; }
    void set_unresolved(bool value) { mask |= 8; unresolved = value; }
    void set_retained_records(unsigned value) { mask |= 16; records = value; }
    std::size_t ByteSize() const {
        return mock::oversized_reply ? 8193 : 128 + nonce.size() + df_version.size()
            + dfhack_version.size() + observation.size() + effect.size();
    }
};
}
