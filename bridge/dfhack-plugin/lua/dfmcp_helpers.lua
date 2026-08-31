-- dfmcp_helpers.lua: Lua bridge reflection and mutation helpers for Dwarf Fortress MCP.
-- Part of WP-DFH-02 DFHack Native Bridge Plugin.

local _ENV = mkmodule('plugins.dfmcp_helpers')

local json = require('json')
local utils = require('utils')

-- Safe pcall wrapper that captures runtime errors into a structured error object.
function safe_execute(fn, ...)
    local success, result_or_err = pcall(fn, ...)
    if not success then
        return {
            ok = false,
            error = tostring(result_or_err),
            error_code = "InternalError"
        }
    end
    return {
        ok = true,
        data = result_or_err
    }
end

-- Get current fortress summary metadata.
function get_fortress_summary()
    local fortress_name = "Unknown Fortress"
    if df.global.world and df.global.world.world_data then
        -- Attempt to read active site/fortress name
        if df.global.ui and df.global.ui.site_name then
            fortress_name = dfhack.TranslateName(df.global.ui.site_name)
        end
    end

    local tick = 0
    local paused = true
    if df.global.cur_year_tick then
        tick = df.global.cur_year_tick
    end
    if df.global.pause_state ~= nil then
        paused = df.global.pause_state
    end

    local pop_count = 0
    if df.global.world and df.global.world.units then
        pop_count = #df.global.world.units.active
    end

    return {
        fortress_id = 1,
        fortress_name = fortress_name,
        tick = tick,
        paused = paused,
        population = pop_count
    }
end

-- Scan active citizens and extract structured unit attributes.
function get_citizen_roster(max_units)
    local limit = max_units or 500
    local citizens = {}
    
    if not df.global.world or not df.global.world.units then
        return citizens
    end

    local count = 0
    for _, unit in ipairs(df.global.world.units.active) do
        if dfhack.units.isCitizen(unit) then
            count = count + 1
            if count > limit then break end

            local name = dfhack.units.getVisibleName(unit)
            local translated_name = name and dfhack.TranslateName(name) or "Unnamed Dwarf"
            local profession = dfhack.units.getProfessionName(unit)

            table.insert(citizens, {
                id = unit.id,
                name = translated_name,
                profession = profession or "Peasant",
                stress = unit.status and unit.status.current_soul and unit.status.current_soul.personality and unit.status.current_soul.personality.stress_level or 0,
                pos = { x = unit.pos.x, y = unit.pos.y, z = unit.pos.z }
            })
        end
    end

    return citizens
end

-- Mutation: set simulation pause state.
function set_pause_state(should_pause)
    if df.global.pause_state ~= nil then
        df.global.pause_state = should_pause
        return { ok = true, paused = df.global.pause_state }
    end
    return { ok = false, error = "pause_state global unavailable" }
end

-- Mutation: designate mining cuboid.
function designate_mining_cuboid(x1, y1, z1, x2, y2, z2, mode_str)
    local min_x = math.min(x1, x2)
    local max_x = math.max(x1, x2)
    local min_y = math.min(y1, y2)
    local max_y = math.max(y1, y2)
    local min_z = math.min(z1, z2)
    local max_z = math.max(z1, z2)

    local tile_mode = df.tile_dig_designation.Default
    if mode_str == "channel" then
        tile_mode = df.tile_dig_designation.Channel
    elseif mode_str == "ramp" then
        tile_mode = df.tile_dig_designation.Ramp
    elseif mode_str == "up_stair" then
        tile_mode = df.tile_dig_designation.UpStair
    elseif mode_str == "down_stair" then
        tile_mode = df.tile_dig_designation.DownStair
    elseif mode_str == "up_down_stair" then
        tile_mode = df.tile_dig_designation.UpDownStair
    end

    local designated_count = 0
    for z = min_z, max_z do
        for y = min_y, max_y do
            for x = min_x, max_x do
                local block = dfhack.maps.getTileBlock(x, y, z)
                if block then
                    local bx = x % 16
                    local by = y % 16
                    block.designation[bx][by].dig = tile_mode
                    block.flags.designation_update = true
                    designated_count = designated_count + 1
                end
            end
        end
    end

    return { ok = true, designated_tiles = designated_count }
end

return _ENV
