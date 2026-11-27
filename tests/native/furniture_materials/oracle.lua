-- Execute the shipped script against a small DFHack API double.
local root = assert(arg[1], 'expected repository root')
local report, printed, suspended
package.preload.json = function()
    return {encode=function(value) report=value; return 'encoded' end}
end
print = function(text) assert(text == 'encoded'); printed=true end
qerror = error
local paused, removed = true, false
dfhack = {
    isWorldLoaded=function() return true end, isMapLoaded=function() return true end,
    getDFVersion=function() return 'df' end, getDFHackVersion=function() return 'dfhack' end,
    with_suspend=function(fn) suspended=true; local value=fn(); suspended=false; return value end,
    world={
        isFortressMode=function() return true end, ReadPauseState=function() return paused end,
        ReadCurrentYear=function() return 1 end, ReadCurrentTick=function() return 2 end,
        ReadWorldFolder=function() return 'region1' end, GetCurrentSiteId=function() return 3 end,
    },
    matinfo={decode=function(t,i)
        assert(suspended and t==419)
        if i==2 then return nil end
        return {type=t,index=i,mode='plant',material={flags={WOOD=i==0}},
                getToken=function() return i==0 and 'PLANT:PECAN:WOOD' or 'PLANT:PECAN:NUT' end}
    end},
}
df = {global={item_next_id=564}, item_type={[7]='BED'}, item={find=function(id)
    assert(suspended)
    return {id=id, flags={removed=removed}, getType=function() return 7 end,
            getSubtype=function() return -1 end, getMaterial=function() return 419 end,
            getMaterialIndex=function() return id-561 end}
end}}
local script = assert(loadfile(root..'/bridge/oracles/dfmcp-furniture-materials.lua'))
script('561','562','563')
assert(printed and report.items[1].resolved and report.items[1].wood_flag)
assert(report.items[2].resolved and not report.items[2].wood_flag)
assert(not report.items[3].resolved and report.items[3].token==nil)
for _, args in ipairs({{}, {'1','1'}, {'1','-2'}, {'1e3'}, {'2147483648'},
                      {'1','2','3','4','5','6','7','8','9'}}) do
    assert(not pcall(script, table.unpack(args)))
end
paused=false; assert(not pcall(script,'561'))
paused=true; removed=true; assert(not pcall(script,'561'))
