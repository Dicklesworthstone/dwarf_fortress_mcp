#pragma once

#include <array>
#include <cstdint>
#include <limits>

#include "df/map_block.h"
#include "modules/Maps.h"

namespace dfmcp_terrain {

// Local to a single suspended capture. A strip of up to 128 tiles can span
// nine 16-wide blocks when its first tile is unaligned. Distinct block columns
// in that strip therefore never collide. Null is an observed missing block.
class BlockRow {
public:
    df::map_block *at(std::uint32_t x, std::uint32_t y, std::uint32_t z) {
        const auto bx = x / 16, by = y / 16;
        auto &entry = blocks_[bx % blocks_.size()];
        if (entry.x != bx || entry.y != by || entry.z != z) {
            entry = {bx, by, z, DFHack::Maps::getTileBlock(x, y, z)};
        }
        return entry.block;
    }

private:
    struct Entry {
        std::uint32_t x = std::numeric_limits<std::uint32_t>::max();
        std::uint32_t y = 0, z = 0;
        df::map_block *block = nullptr;
    };
    std::array<Entry, 9> blocks_{};
};

} // namespace dfmcp_terrain
