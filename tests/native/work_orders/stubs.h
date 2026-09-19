#pragma once
// Explicit DFHack/protobuf boundary doubles. This is not a native SDK build.
#include <cstdint>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

namespace df {
enum class job_type : std::int16_t { NONE = -1, ConstructDoor = 67, ConstructBed = 69, ConstructThrone = 70, ConstructTable = 72 };
enum class item_type : std::int16_t { NONE = -1, BED = 1 };
enum class workquota_frequency_type : std::int32_t { NONE = -1, OneTime = 0, Daily = 1 };
inline unsigned live_orders = 0;
struct manager_order {
    std::int32_t id = 0;
    df::job_type job_type = df::job_type::NONE;
    df::item_type item_type = df::item_type::NONE;
    std::int16_t item_subtype = -1, mat_type = 0;
    std::int32_t mat_index = -1;
    std::string reaction_name;
    struct { struct { std::uint32_t whole = 0; } encrust_flags; } specflag;
    struct { std::int32_t hist_figure_id = 0; } specdata;
    union Category {
        std::uint32_t whole;
        struct { unsigned wood : 1; unsigned other : 31; } bits;
        Category() : whole(0) {}
    } material_category;
    struct { std::int32_t type = -1, id = -1, subid = -1; } art_spec;
    std::int16_t amount_left = 0, amount_total = 0;
    struct { std::uint32_t whole = 0; } status;
    df::workquota_frequency_type frequency = df::workquota_frequency_type::NONE;
    std::int32_t finished_year = -1, finished_year_tick = -1, workshop_id = 0, max_workshops = 0;
    std::vector<int> item_conditions, order_conditions;
    void *items = nullptr;
    manager_order() { ++live_orders; }
    ~manager_order() { --live_orders; }
    manager_order(const manager_order &) = delete;
    manager_order &operator=(const manager_order &) = delete;
};
inline bool fail_reserve = false;
struct OrderVector : std::vector<manager_order *> {
    void reserve(std::size_t n) { if (fail_reserve) throw std::bad_alloc(); std::vector<manager_order *>::reserve(n); }
};
struct world { struct { OrderVector all; std::int32_t manager_order_next_id = 0; } manager_orders; };
namespace global { inline df::world *world = nullptr; }
}
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK = 0, CR_FAILURE = 1 };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_PAUSED, SC_UNPAUSED, SC_VIEWSCREEN_CHANGED };
struct VersionInfo { std::string getVersion() const { return "test-df"; } };
struct Core {
    std::shared_ptr<VersionInfo> vinfo = std::make_shared<VersionInfo>();
    bool world_loaded = true, map_loaded = true;
    static Core &getInstance() { static Core core; return core; }
    bool isWorldLoaded() const { return world_loaded; }
    bool isMapLoaded() const { return map_loaded; }
};
namespace Version { inline const char *dfhack_version() { return "test-dfhack"; } }
namespace World {
inline std::int64_t year = 0;
inline std::uint32_t tick = 12345;
inline std::int32_t site = 1;
inline bool fortress = true, paused = true;
inline std::string folder = "region1";
inline std::int64_t ReadCurrentYear() { return year; }
inline std::uint32_t ReadCurrentTick() { return tick; }
inline std::int32_t GetCurrentSiteId() { return site; }
inline std::string ReadWorldFolder() { return folder; }
inline bool isFortressMode() { return fortress; }
inline bool ReadPauseState() { return paused; }
}
struct RPCService {
    std::vector<std::string> names;
    std::vector<int> flags;
    template<class Function> void addFunction(const char *name, Function, int flag) { names.emplace_back(name); flags.push_back(flag); }
};
}
#define DFHACK_PLUGIN(name) static_assert(sizeof(name) > 1, "plugin name")
#define DFhackCExport

namespace dfmcp::work_orders::v1_10 {
#define WO_FIELD(type, name) \
private: type name##_{}; bool has_##name##_ = false; \
public: const type &name() const { return name##_; } bool has_##name() const { return has_##name##_; } \
void set_##name(type value) { name##_ = std::move(value); has_##name##_ = true; } \
void clear_##name() { name##_ = {}; has_##name##_ = false; }
struct UnknownFields { bool present = false; bool empty() const { return !present; } };
struct Request {
    WO_FIELD(std::string, bearer_token)
    WO_FIELD(std::string, client_nonce)
    WO_FIELD(std::uint32_t, protocol_major)
    WO_FIELD(std::uint32_t, protocol_minor)
    WO_FIELD(std::string, idempotency_key)
    WO_FIELD(std::uint32_t, recipe)
    WO_FIELD(std::uint32_t, amount)
    WO_FIELD(std::string, expected_witness)
    WO_FIELD(std::string, plan_digest)
    WO_FIELD(std::string, prepare_token)
    UnknownFields unknown;
    struct Reflection { const UnknownFields &GetUnknownFields(const Request &in) const { return in.unknown; } } reflection;
    const Reflection *GetReflection() const { return &reflection; }
    bool IsInitialized() const { return has_bearer_token() && has_client_nonce() && has_protocol_major() && has_protocol_minor(); }
};
inline bool fail_reply = false;
struct Reply {
    WO_FIELD(bool, accepted)
    WO_FIELD(std::uint32_t, failure_code)
    WO_FIELD(std::string, client_nonce)
    WO_FIELD(std::uint32_t, protocol_major)
    WO_FIELD(std::uint32_t, protocol_minor)
    WO_FIELD(std::uint64_t, bridge_generation)
    WO_FIELD(std::string, df_version)
    WO_FIELD(std::string, dfhack_version)
    WO_FIELD(std::string, observation)
    WO_FIELD(bool, replayed)
private: std::string effect_record_; bool has_effect_record_ = false;
public:
    const std::string &effect_record() const { return effect_record_; }
    bool has_effect_record() const { return has_effect_record_; }
    void set_effect_record(const std::string &value) {
        if (fail_reply) { fail_reply = false; throw std::bad_alloc(); }
        effect_record_ = value; has_effect_record_ = true;
    }
    void Clear() { *this = Reply(); }
};
#undef WO_FIELD
}
