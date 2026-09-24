#pragma once
#include "../common/excavation_run_wire.h"
#include "Core.h"
#include "TileTypes.h"
#include "modules/Maps.h"
#include "modules/World.h"
#include "df/map_block.h"
#include "df/tile_designation.h"
#include "df/tiletype_shape.h"

namespace dfmcp_excavation_run_native {
namespace run = dfmcp_excavation_run;
inline run::Identity identity(std::uint64_t generation) {
    using namespace DFHack;
    run::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode(), 4);
    const auto site = World::GetCurrentSiteId(); run::require(site >= 0 && site <= INT32_MAX, 5);
    std::int32_t x = 0, y = 0, z = 0; Maps::getTileSize(x, y, z);
    run::require(x >= 1 && x <= 32768 && y >= 1 && y <= 32768 && z >= 1 && z <= 32768, 5);
    run::Identity out{generation, static_cast<std::uint32_t>(site), static_cast<std::uint32_t>(x),
        static_cast<std::uint32_t>(y), static_cast<std::uint32_t>(z), World::ReadWorldFolder()};
    out.validate(); return out;
}
inline run::Source source(std::uint64_t generation) {
    using namespace DFHack;
    run::Source out; out.identity = identity(generation);
    out.clock = {generation, 0, 0, true, false, World::ReadPauseState()};
    // A bad game clock does not erase independently observed pause state.
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = static_cast<std::int64_t>(World::ReadCurrentTick());
    if (year >= 0 && year <= UINT32_MAX && tick >= 0 && tick < 403200) {
        out.clock.clock_valid = true;
        out.clock.tick = static_cast<std::uint64_t>(year) * 403200 + static_cast<std::uint64_t>(tick);
    }
    out.validate(); return out;
}
inline std::uint8_t shape_tag(df::tiletype_shape shape) {
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
inline run::Capture capture(std::uint64_t generation, run::Region region) {
    using namespace DFHack;
    region.validate(); run::Capture out; out.region = region; out.source = source(generation);
    const auto &id = out.source.identity;
    run::require(out.source.clock.usable() && region.x + region.width <= id.map_x
        && region.y + region.height <= id.map_y && region.z < id.map_z, 5);
    for (std::uint32_t dy = 0; dy < region.height; ++dy)
    for (std::uint32_t dx = 0; dx < region.width; ++dx) {
        auto &cell = out.cells[dy * region.width + dx];
        const auto x = region.x + dx, y = region.y + dy;
        auto *block = Maps::getTileBlock(static_cast<std::int32_t>(x), static_cast<std::int32_t>(y), static_cast<std::int32_t>(region.z));
        if (!block) continue;
        const auto lx = x & 15, ly = y & 15;
        const auto &bits = block->designation[lx][ly].bits;
        if (bits.hidden) { cell.presence = 1; continue; }
        // Hidden cells never read tiletype, shape, liquid or designation payload.
        const auto tile = block->tiletype[lx][ly];
        run::require(static_cast<int>(tile) >= 0 && is_valid_enum_item(tile), 5);
        cell = {2, shape_tag(tileShape(tile)), static_cast<std::uint8_t>(bits.flow_size), static_cast<std::uint8_t>(bits.dig)};
    }
    out.validate(); return out;
}
inline void set_pause(std::uint64_t generation, const run::Identity &expected, bool paused) {
    // No terrain/clock acquisition here: even failed capture must be stoppable.
    run::require(identity(generation) == expected, 4);
    DFHack::World::SetPauseState(paused);
}
} // namespace dfmcp_excavation_run_native
