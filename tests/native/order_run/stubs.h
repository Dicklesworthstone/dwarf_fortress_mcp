#pragma once
// Explicit SDK doubles. They do NOT establish a real DFHack ABI or plugin build.
#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace df {
enum class job_type : std::int16_t { ConstructBed = 69, ConstructDoor = 67, ConstructTable = 72, ConstructThrone = 70, CustomReaction = 1 };
enum class item_type { NONE = -1, BED = 0 };
enum class workquota_frequency_type { OneTime = 0, Daily = 1 };
union Category { std::uint32_t whole = 0; struct { std::uint32_t wood:1; std::uint32_t rest:31; } bits; };
union Status { std::uint32_t whole = 0; struct { std::uint32_t validated:1; std::uint32_t active:1; std::uint32_t rest:30; } bits; };
struct manager_order {
    std::int32_t id = 0;
    df::job_type job_type = df::job_type::ConstructBed;
    df::item_type item_type = df::item_type::NONE;
    std::int16_t item_subtype = -1, mat_type = -1;
    std::int32_t mat_index = -1;
    std::string reaction_name;
    struct { struct { std::uint32_t whole = 0; } encrust_flags; } specflag;
    struct { std::int32_t hist_figure_id = -1; } specdata;
    Category material_category;
    struct { int type = 0, id = -1, subid = -1; } art_spec;
    std::int16_t amount_left = 10, amount_total = 10;
    Status status;
    df::workquota_frequency_type frequency = df::workquota_frequency_type::OneTime;
    std::int32_t workshop_id = -1, max_workshops = 1;
    std::vector<int *> item_conditions, order_conditions;
    void *items = nullptr;
};
struct world { struct { std::vector<manager_order *> all; std::int32_t manager_order_next_id = 10; } manager_orders; };
namespace global { inline df::world *world = nullptr; }
}
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_BEGIN_UNLOAD };
inline int suspension_depth = 0;
struct CoreSuspender { CoreSuspender() { ++suspension_depth; } ~CoreSuspender() { --suspension_depth; } };
inline void require_suspended() { if (!suspension_depth) throw std::runtime_error("unscoped native access"); }
struct VersionInfo { std::string getVersion() const { return "test-df"; } };
struct Core {
    VersionInfo version; VersionInfo *vinfo = &version;
    bool loaded = true, map = true;
    static Core &getInstance() { static Core core; return core; }
    bool isWorldLoaded() const { return loaded; }
    bool isMapLoaded() const { return map; }
};
namespace Version { inline const char *dfhack_version() { return "test-dfhack"; } }
namespace World {
inline bool fortress = true, paused = true;
inline std::int64_t year = 0, site = 7, tick = 100;
inline std::string folder = "fort";
inline int unpauses = 0, pauses = 0, reads = 0;
inline bool fail_pause = false, fail_after_unpause = false;
inline void (*during_unpause)() = nullptr;
inline bool isFortressMode() { return fortress; }
inline auto ReadCurrentYear() { require_suspended(); ++reads; return year; }
inline auto ReadCurrentTick() { require_suspended(); return tick; }
inline auto GetCurrentSiteId() { require_suspended(); return site; }
inline bool ReadPauseState() { require_suspended(); return paused; }
inline auto ReadWorldFolder() { require_suspended(); return folder; }
inline void SetPauseState(bool p) {
    require_suspended();
    if (p) { ++pauses; if (fail_pause) throw std::runtime_error("pause failure"); }
    else { ++unpauses; if (during_unpause) during_unpause(); }
    paused = p;
    if (!p && fail_after_unpause) throw std::runtime_error("ambiguous unpause");
}
}
struct RPCService {
    std::vector<std::pair<std::string,int>> methods;
    template<class F> void addFunction(const std::string &name, F, int flags) { methods.emplace_back(name, flags); }
};
}
