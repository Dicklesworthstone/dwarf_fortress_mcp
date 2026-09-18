#pragma once

// Deliberately small test doubles. These do NOT qualify DFHack ABI, protobuf
// serialization, RPC suspension, the Rust coordinator or a real game process.
#include <cstdint>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace fake {
inline bool paused = true, loaded = true, fortress = true;
inline std::uint32_t year = 105, tick = 3;
inline unsigned setters = 0;
enum class Fault { None, SetterBefore, SetterAfter, Readback, ClockAfter, NoEffect };
inline Fault fault = Fault::None;
}
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK };
enum state_change_event { SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_MAP_LOADED, SC_MAP_UNLOADED, SC_PAUSED };
struct VersionInfo { std::string getVersion() const { return "fixture-df"; } };
struct Core {
    std::shared_ptr<VersionInfo> vinfo = std::make_shared<VersionInfo>();
    static Core &getInstance() { static Core core; return core; }
    bool isWorldLoaded() const { return fake::loaded; }
};
namespace Version { inline std::string dfhack_version() { return "fixture-dfhack"; } }
namespace World {
inline bool isFortressMode() { return fake::fortress; }
inline std::uint32_t ReadCurrentYear() { return fake::year; }
inline std::uint32_t ReadCurrentTick() {
    return fake::fault == fake::Fault::ClockAfter && fake::setters ? 403200 : fake::tick;
}
inline bool ReadPauseState() {
    if (fake::fault == fake::Fault::Readback && fake::setters) throw std::runtime_error("readback failure");
    return fake::paused;
}
inline void SetPauseState(bool paused) {
    ++fake::setters;
    if (fake::fault == fake::Fault::SetterBefore) throw std::runtime_error("setter failure before effect");
    if (fake::fault == fake::Fault::NoEffect) return;
    fake::paused = paused;
    if (fake::fault == fake::Fault::SetterAfter) throw std::runtime_error("setter failure after effect");
}
}
struct RPCService {
    std::vector<std::string> methods;
    template<typename F> void addFunction(const char *name, F, unsigned flags) {
        if (flags != 0) throw std::runtime_error("test requires suspended local RPCs");
        methods.emplace_back(name);
    }
};
}
namespace dfmcp::control::v1_7 {
#define STRING_FIELD(name) \
private: std::string name##_; bool has_##name##_ = false; \
public: const std::string &name() const { return name##_; } \
    void set_##name(const std::string &v) { name##_ = v; has_##name##_ = true; } \
    bool has_##name() const { return has_##name##_; }
#define NUMBER_FIELD(type, name) \
private: type name##_{}; bool has_##name##_ = false; \
public: type name() const { return name##_; } \
    void set_##name(type v) { name##_ = v; has_##name##_ = true; } \
    bool has_##name() const { return has_##name##_; }
class Request {
    STRING_FIELD(bearer_token)
    STRING_FIELD(client_nonce)
    NUMBER_FIELD(std::uint32_t, protocol_major)
    NUMBER_FIELD(std::uint32_t, protocol_minor)
    STRING_FIELD(idempotency_key)
    STRING_FIELD(plan_digest)
    STRING_FIELD(prepare_token)
    NUMBER_FIELD(bool, paused)
    NUMBER_FIELD(std::uint64_t, expected_game_tick)
    NUMBER_FIELD(bool, query_only)
};
class Reply {
public: void Clear() { *this = Reply{}; }
    NUMBER_FIELD(bool, accepted)
    NUMBER_FIELD(std::uint32_t, failure_code)
    STRING_FIELD(client_nonce)
    NUMBER_FIELD(std::uint32_t, protocol_major)
    NUMBER_FIELD(std::uint32_t, protocol_minor)
    NUMBER_FIELD(std::uint64_t, bridge_generation)
    STRING_FIELD(df_version)
    STRING_FIELD(dfhack_version)
    STRING_FIELD(prepare_token)
    NUMBER_FIELD(bool, effect_known)
    NUMBER_FIELD(bool, effect_applied)
    NUMBER_FIELD(bool, paused)
    NUMBER_FIELD(std::uint64_t, observed_game_tick)
    STRING_FIELD(receipt_digest)
};
#undef STRING_FIELD
#undef NUMBER_FIELD
}
