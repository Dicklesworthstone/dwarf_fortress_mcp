#pragma once
// Explicit SDK/protobuf test doubles. This does not establish a real plugin build.
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
    std::int16_t amount_left = 5, amount_total = 5;
    Status status;
    df::workquota_frequency_type frequency = df::workquota_frequency_type::OneTime;
    std::int32_t finished_year = -1, finished_year_tick = -1, workshop_id = -1, max_workshops = 1;
    std::vector<int *> item_conditions, order_conditions;
    void *items = nullptr;
};
struct world { struct { std::vector<manager_order *> all; std::int32_t manager_order_next_id = 10; } manager_orders; };
namespace global { inline df::world *world = nullptr; }
}
inline std::string test_enum_key(df::job_type t) {
    switch (t) {
        case df::job_type::ConstructBed: return "ConstructBed";
        case df::job_type::ConstructDoor: return "ConstructDoor";
        case df::job_type::ConstructTable: return "ConstructTable";
        case df::job_type::ConstructThrone: return "ConstructThrone";
        default: return "CustomReaction";
    }
}
#define ENUM_KEY_STR(kind, value) test_enum_key(value)
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_PAUSED, SC_UNPAUSED };
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
inline std::int64_t year = 0, site = 1;
inline std::int64_t tick = 12345;
inline std::string folder = "region1";
inline bool isFortressMode() { return fortress; }
inline auto ReadCurrentYear() { return year; }
inline auto ReadCurrentTick() { return tick; }
inline auto GetCurrentSiteId() { return site; }
inline bool ReadPauseState() { return paused; }
inline auto ReadWorldFolder() { return folder; }
}
struct RPCService {
    std::vector<std::pair<std::string,int>> methods;
    template<class F> void addFunction(const std::string &name, F, int flags) { methods.emplace_back(name, flags); }
};
}
namespace dfmcp::work_order_progress::v1_12 {
struct Request {
    std::string token = std::string(32, 'x'), nonce = std::string(16, 'n');
    std::uint32_t major = 1, minor = 12;
    std::vector<std::uint32_t> ids;
    bool initialized = true, unknown = false;
    struct Fields { bool is_empty; bool empty() const { return is_empty; } };
    struct Reflection { Fields GetUnknownFields(const Request &r) const { return {!r.unknown}; } } reflection;
    bool IsInitialized() const { return initialized; }
    const Reflection *GetReflection() const { return &reflection; }
    const auto &bearer_token() const { return token; }
    const auto &client_nonce() const { return nonce; }
    auto protocol_major() const { return major; }
    auto protocol_minor() const { return minor; }
    int native_order_ids_size() const { return static_cast<int>(ids.size()); }
    const auto &native_order_ids() const { return ids; }
};
struct Reply {
    bool accepted = false, has_observation = false, fail_observation = false;
    std::uint32_t code = 0, major = 0, minor = 0;
    std::uint64_t generation = 0;
    std::string nonce, df, dfhack, observation;
    void Clear() { bool fail = fail_observation; *this = Reply{}; fail_observation = fail; }
    void set_accepted(bool v) { accepted = v; }
    void set_failure_code(std::uint32_t v) { code = v; }
    void set_protocol_major(std::uint32_t v) { major = v; }
    void set_protocol_minor(std::uint32_t v) { minor = v; }
    void set_bridge_generation(std::uint64_t v) { generation = v; }
    void set_client_nonce(std::string v) { nonce = std::move(v); }
    void set_df_version(std::string v) { df = std::move(v); }
    void set_dfhack_version(std::string v) { dfhack = std::move(v); }
    void set_observation(std::string v) {
        if (fail_observation) throw std::bad_alloc();
        observation = std::move(v); has_observation = true;
    }
};
}
