-- Inspect material raws for the selected item IDs in a paused fortress.
-- Usage: dfmcp-furniture-materials <native-item-id> [<native-item-id> ...]
local json = require('json')
local args = {...}
if #args < 1 or #args > 8 then qerror('expected 1..8 selected furniture item IDs') end

local ids = {}
for _, arg in ipairs(args) do
    local id = tonumber(arg)
    if not arg:match('^%d+$') or not id or id > 2147483647
            or (#ids > 0 and id <= ids[#ids]) then
        qerror('expected increasing nonnegative native item IDs')
    end
    table.insert(ids, id)
end

local function observe()
    local world = dfhack.world
    if not dfhack.isWorldLoaded() or not dfhack.isMapLoaded()
            or not world.isFortressMode() or not world.ReadPauseState() then
        qerror('a loaded paused fortress is required')
    end
    local report = {
        schema='dfmcp.furniture_material_observation/1',
        df_version=dfhack.getDFVersion(), dfhack_version=dfhack.getDFHackVersion(),
        world_folder=world.ReadWorldFolder(), site_id=world.GetCurrentSiteId(), paused=true,
        year=world.ReadCurrentYear(), year_tick=world.ReadCurrentTick(),
        next_item_id=df.global.item_next_id, items={},
    }
    for _, id in ipairs(ids) do
        local item = df.item.find(id)
        if not item or item.id ~= id or item.flags.removed then
            qerror('selected item is absent or removed: '..id)
        end
        local kind = df.item_type[item:getType()]
        if kind ~= 'BED' and kind ~= 'CHAIR' and kind ~= 'DOOR' and kind ~= 'TABLE' then
            qerror('selected item is not a bed, chair, door or table: '..id)
        end
        local mat_type, mat_index = item:getMaterial(), item:getMaterialIndex()
        local row = {native_item_id=id, type_key=kind, subtype=item:getSubtype(),
            material={mat_type, mat_index}, resolved=false}
        local info = dfhack.matinfo.decode(mat_type, mat_index)
        if info and info.material and info.type == mat_type and info.index == mat_index then
            local flag = info.material.flags.WOOD
            local token = info:getToken()
            if type(flag) == 'boolean' and type(token) == 'string' and #token > 0
                    and #token <= 512 and not token:find('%c') then
                row.resolved, row.mode, row.wood_flag, row.token = true, info.mode, flag, token
            end
        end
        table.insert(report.items, row)
    end
    return report
end

print(json.encode(dfhack.with_suspend(observe)))
