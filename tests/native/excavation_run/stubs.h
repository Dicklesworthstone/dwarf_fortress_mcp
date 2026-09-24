#pragma once
// Explicit SDK doubles. These are not real DFHack structures or ABI evidence.
#include <array>
#include <cstdint>
#include <functional>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
namespace fake {
inline int suspension = 0, suspension_entries = 0, shape_reads = 0, tile_reads = 0;
inline int pauses = 0, unpauses = 0;
inline bool loaded = true, map_loaded = true, fortress = true, paused = true;
inline bool fail_capture = false, fail_pause = false, noop_pause = false, fail_unpause = false, throw_reply = false;
inline std::int64_t year = 0, tick = 100;
inline std::int32_t site = 2, sx = 64, sy = 64, sz = 8;
inline std::string folder = "region1";
inline std::function<void(bool)> setter_hook;
inline void suspended() { if (suspension < 1) throw std::runtime_error("native access outside suspension"); }
}
namespace df {
enum class tiletype_shape { EMPTY, WALL, FLOOR, RAMP, RAMP_TOP, STAIR_UP, STAIR_DOWN, STAIR_UPDOWN, OTHER };
enum class tiletype { EMPTY, WALL, FLOOR, RAMP, RAMP_TOP, STAIR_UP, STAIR_DOWN, STAIR_UPDOWN, OTHER };
struct tile_designation { struct { unsigned hidden = 0, flow_size = 0, dig = 0; } bits; };
struct map_block { tile_designation designation[16][16]{}; df::tiletype tiletype[16][16]{}; };
}
namespace fake {
inline df::map_block blocks[4][4];
inline bool missing[4][4]{};
inline void terrain(bool floor) {
    for (auto &column : blocks) for (auto &block : column)
        for (unsigned x = 0; x < 16; ++x) for (unsigned y = 0; y < 16; ++y) {
            block.tiletype[x][y] = floor ? df::tiletype::FLOOR : df::tiletype::WALL;
            block.designation[x][y].bits = {0, 0, floor ? 0u : 1u};
        }
    for (auto &column : missing) for (auto &entry : column) entry = false;
}
}
inline bool is_valid_enum_item(df::tiletype t) { return static_cast<int>(t) >= 0 && static_cast<int>(t) <= 8; }
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_BEGIN_UNLOAD, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_MAP_LOADED, SC_MAP_UNLOADED, SC_PAUSED, SC_UNPAUSED };
struct VersionInfo { std::string getVersion() const { return "fake-df"; } };
struct Core {
    std::shared_ptr<VersionInfo> vinfo = std::make_shared<VersionInfo>();
    static Core &getInstance() { static Core value; return value; }
    bool isWorldLoaded() const { fake::suspended(); return fake::loaded; }
    bool isMapLoaded() const { fake::suspended(); return fake::map_loaded; }
};
struct CoreSuspender {
    CoreSuspender() { ++fake::suspension; ++fake::suspension_entries; }
    ~CoreSuspender() { --fake::suspension; }
};
namespace Version { inline const char *dfhack_version() { return "fake-dfhack"; } }
namespace Maps {
inline void getTileSize(std::int32_t &x, std::int32_t &y, std::int32_t &z) { fake::suspended(); x = fake::sx; y = fake::sy; z = fake::sz; }
inline df::map_block *getTileBlock(std::int32_t x, std::int32_t y, std::int32_t z) {
    fake::suspended(); ++fake::tile_reads;
    if (fake::fail_capture) throw std::runtime_error("map capture failed");
    if (x < 0 || y < 0 || z < 0 || x >= fake::sx || y >= fake::sy || z >= fake::sz || x >= 64 || y >= 64)
        throw std::runtime_error("out of bounds native tile");
    if (fake::missing[x / 16][y / 16]) return nullptr;
    return &fake::blocks[x / 16][y / 16];
}
}
inline df::tiletype_shape tileShape(df::tiletype value) { fake::suspended(); ++fake::shape_reads; return static_cast<df::tiletype_shape>(value); }
namespace World {
inline bool isFortressMode() { fake::suspended(); return fake::fortress; }
inline std::int32_t GetCurrentSiteId() { fake::suspended(); return fake::site; }
inline std::string ReadWorldFolder() { fake::suspended(); return fake::folder; }
inline bool ReadPauseState() { fake::suspended(); return fake::paused; }
inline std::int64_t ReadCurrentYear() { fake::suspended(); return fake::year; }
inline std::int64_t ReadCurrentTick() { fake::suspended(); return fake::tick; }
inline void SetPauseState(bool paused) {
    fake::suspended();
    if (paused) { ++fake::pauses; if (fake::fail_pause) throw std::runtime_error("pause failed"); }
    else ++fake::unpauses;
    if (!(paused && fake::noop_pause)) fake::paused = paused;
    if (fake::setter_hook) fake::setter_hook(paused);
    if (!paused && fake::fail_unpause) throw std::runtime_error("unpause reply lost");
}
}
struct RPCService {
    std::vector<std::string> names;
    std::vector<unsigned> flags;
    template<class Function> void addFunction(const char *name, Function, unsigned flag) { names.emplace_back(name); flags.push_back(flag); }
};
}
#define DFHACK_PLUGIN(name)
#define DFhackCExport
