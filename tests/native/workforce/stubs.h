#pragma once
// Explicit DFHack/protobuf API doubles. No real SDK, protobuf or game behavior.
#include <array>
#include <algorithm>
#include <cstdint>
#include <functional>
#include <stdexcept>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace df {
enum class unit_labor { MINE, CARPENTER, HAUL };
enum class work_detail_mode { EverybodyDoesThis, OnlySelectedDoesThis, NobodyDoesThis };
struct work_detail {
    std::string name;
    struct { std::uint32_t whole = 2; struct { work_detail_mode mode = work_detail_mode::OnlySelectedDoesThis; } bits; } flags;
    std::vector<std::int32_t> assigned_units;
    std::array<bool,3> allowed_labors{};
};
struct unit {
    std::int32_t id = 0, hist_figure_id = 0;
    bool active = true, citizen = true, sane = true, adult = true;
    struct { std::array<bool,3> labors{}; } status;
};
struct world { struct { std::vector<unit *> active; } units; };
struct plotinfost { struct { std::vector<work_detail *> work_details; } labor_info; };
struct gamest { struct { struct { bool automatic_professions_disabled = false; } bits; } external_flag; };
namespace global { inline world *world = nullptr; inline plotinfost *plotinfo = nullptr; inline gamest *game = nullptr; }
}
namespace DFHack {
struct color_ostream {}; struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_MAP_LOADED, SC_MAP_UNLOADED, SC_PAUSED };
inline int suspended = 0;
inline void require_suspend() { if (!suspended) throw std::runtime_error("native read outside suspension"); }
struct CoreSuspender { CoreSuspender() { ++suspended; } ~CoreSuspender() { --suspended; } };
struct VersionInfo { std::string getVersion() const { return "test-df"; } };
struct Core {
    VersionInfo version; VersionInfo *vinfo = &version; bool loaded = true, map = true;
    static Core &getInstance() { static Core instance; return instance; }
    bool isWorldLoaded() const { require_suspend(); return loaded; }
    bool isMapLoaded() const { require_suspend(); return map; }
};
namespace Version { inline const char *dfhack_version() { return "test-dfhack"; } }
namespace World {
inline bool fortress = true, paused = true;
inline std::int64_t year = 0, tick = 100;
inline std::int32_t site = 7;
inline std::string folder = "region1";
inline bool isFortressMode() { require_suspend(); return fortress; }
inline auto ReadCurrentYear() { require_suspend(); return year; }
inline auto ReadCurrentTick() { require_suspend(); return tick; }
inline auto GetCurrentSiteId() { require_suspend(); return site; }
inline auto ReadWorldFolder() { require_suspend(); return folder; }
inline bool ReadPauseState() { require_suspend(); return paused; }
}
namespace Units {
inline int calls = 0;
inline std::function<void(df::unit *)> hook;
inline bool isActive(df::unit *u) { require_suspend(); return u->active; }
inline bool isCitizen(df::unit *u, bool insane = false) { require_suspend(); return u->citizen && (insane || u->sane); }
inline bool isAdult(df::unit *u) { require_suspend(); return u->adult; }
inline void setAutomaticProfessions(df::unit *u) {
    require_suspend(); ++calls;
    if (hook) { hook(u); return; }
    u->status.labors.fill(false);
    for (auto *d : df::global::plotinfo->labor_info.work_details) {
        const bool selected = std::find(d->assigned_units.begin(),d->assigned_units.end(),u->id)!=d->assigned_units.end();
        if (selected) for (std::size_t i=0;i<3;++i) u->status.labors[i] = u->status.labors[i] || d->allowed_labors[i];
    }
}
}
inline std::string DF2UTF(const std::string &v) { return v; }
struct RPCService {
    std::vector<std::string> names;
    template<class F> void addFunction(const std::string &n, F, int flags) {
        if (flags) throw std::runtime_error("unsuspended registration");
        names.push_back(n);
    }
};
}
inline std::string enum_key(df::unit_labor v) {
    switch(v) { case df::unit_labor::MINE:return "MINE"; case df::unit_labor::CARPENTER:return "CARPENTER";
        case df::unit_labor::HAUL:return "HAUL"; } return "UNKNOWN";
}
#define ENUM_KEY_STR(kind,value) enum_key(value)
namespace dfmcp::workforce::v1_17 {
struct Request {
    std::string token=std::string(32,'t'), nonce=std::string(16,'n'), key, witness, plan, prepare;
    std::uint32_t major=1,minor=17,index=0; bool desired=true;
    std::vector<std::uint32_t> ids;
    unsigned mask=0; bool initialized=true; int unknown=0; std::size_t size=300;
    struct Fields { int n; int field_count() const { return n; } };
    struct Reflection { Fields GetUnknownFields(const Request &r) const { return {r.unknown}; } } reflection;
    const Reflection *GetReflection() const { return &reflection; }
    bool IsInitialized() const { return initialized; } std::size_t ByteSizeLong() const { return size; }
    const auto &bearer_token() const { return token; } const auto &client_nonce() const { return nonce; }
    auto protocol_major() const { return major; } auto protocol_minor() const { return minor; }
    bool has_idempotency_key() const { return mask&1; } const auto &idempotency_key() const { return key; }
    bool has_detail_index() const { return mask&2; } auto detail_index() const { return index; }
    bool has_assigned() const { return mask&4; } bool assigned() const { return desired; }
    bool has_expected_witness() const { return mask&8; } const auto &expected_witness() const { return witness; }
    bool has_plan_digest() const { return mask&16; } const auto &plan_digest() const { return plan; }
    bool has_prepare_token() const { return mask&32; } const auto &prepare_token() const { return prepare; }
    const auto &unit_ids() const { return ids; } int unit_ids_size() const { return static_cast<int>(ids.size()); }
};
struct Reply {
    bool accepted=false, unresolved=false, effect_present=false, observation_present=false, fail_effect=false;
    std::uint32_t code=0,major=0,minor=0,count=0; std::uint64_t generation=0;
    std::string nonce,df,dfhack,observation,effect;
    void Clear() { bool fail=fail_effect; *this=Reply{}; fail_effect=fail; }
    void set_accepted(bool v) { accepted=v; } void set_failure_code(std::uint32_t v) { code=v; }
    void set_client_nonce(std::string v) { nonce=std::move(v); }
    void set_protocol_major(std::uint32_t v) { major=v; } void set_protocol_minor(std::uint32_t v) { minor=v; }
    void set_bridge_generation(std::uint64_t v) { generation=v; }
    void set_df_version(std::string v) { df=std::move(v); } void set_dfhack_version(std::string v) { dfhack=std::move(v); }
    void set_observation(std::string v) { observation=std::move(v); observation_present=true; }
    void set_effect_record(std::string v) { if(fail_effect) throw std::bad_alloc(); effect=std::move(v); effect_present=true; }
    void set_unresolved(bool v) { unresolved=v; } void set_retained_records(std::uint32_t v) { count=v; }
};
}
