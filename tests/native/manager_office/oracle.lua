-- Run with Lua 5.4: lua tests/native/manager_office/oracle.lua .
local root = assert(arg[1])
local script = assert(loadfile(root..'/bridge/oracles/dfmcp-manager-office.lua'))
local suspended, report, paused, xml
local group, position, profile, hf, unit, roles, office, chair, other
local function guarded(f)
    return function(...) assert(suspended, 'game read outside suspension'); return f(...) end
end
local function const(v) return guarded(function() return v end) end
package.preload.json = function()
    return {encode=function(r, options)
        assert(not suspended and not options.pretty)
        report=r
        return 'one observation'
    end}
end
print = function(s) assert(s == 'one observation') end
qerror = error
local function make_chair(id, x, y, z)
    return {id=id, centerx=x, centery=y, z=z, stage=1, flags={exists=true, almost_deleted=false},
        jobs={}, getType=const(3), getMaxBuildStage=const(1),
        getBuildStage=guarded(function(self) return self.stage end)}
end
local function make_office(id)
    return {id=id, type=9, assigned_unit_id=256, z=2, active=true, flags={almost_deleted=false},
        room={extents={}}, contained_buildings={}, isExtentShaped=const(true)}
end
local function reset()
    report=nil
    paused, xml=true, '1dd01aad64219afa0578f1328cf15bd0c6006d5a'
    position={id=7, code='MANAGER', required_office=1, responsibilities={[12]=true}}
    -- histfig2 is the previous holder and may differ.
    profile={id=11, position_id=7, histfig=361, histfig2=300}
    group={id=4, positions={own={position}, assignments={profile}}}
    hf={id=361, unit_id=256, entity_links={}}
    office, chair=make_office(80), make_chair(81, 5, 6, 2)
    office.contained_buildings={chair}
    unit={id=256, hist_figure_id=361, owned_buildings={office}, job={current_job={id=19,job_type=43}}}
    roles={{entity=group, position=position, assignment=profile}}
    other={ZONE_OFFICE={office}, CHAIR={chair}}
    df={global={world={buildings={other=other}}, plotinfo={group_id=4,
            nobles={manager_cooldown=733}, manager_timer=13}},
        historical_entity={find=guarded(function(id) if id==group.id then return group end end)},
        historical_figure={find=guarded(function(id) if id==hf.id then return hf end end)},
        unit={find=guarded(function(id) if id==unit.id then return unit end end)},
        isnull=function(v) return v==nil end, entity_position_responsibility={MANAGE_PRODUCTION=12},
        civzone_type={Office=9}, building_type={Chair=3}, job_type={[43]='ManageWorkOrders'}}
    dfhack={
        getDFHackVersion=const('53.16-r1.1'), getDFVersion=const('v0.53.16 linux64 CLASSIC'),
        getGitXmlCommit=guarded(function() return xml end),
        isWorldLoaded=const(true), isMapLoaded=const(true),
        with_suspend=function(f)
            assert(not suspended)
            suspended=true
            local ok, value=pcall(f)
            suspended=false
            if not ok then error(value) end
            return value
        end,
        world={isFortressMode=const(true), ReadPauseState=guarded(function() return paused end),
            ReadWorldFolder=const('region1'), GetCurrentSiteId=const(17),
            ReadCurrentYear=const(93), ReadCurrentTick=const(2000)},
        units={getNoblePositions=guarded(function(u) assert(u==unit); return roles end),
            isActive=const(true), isCitizen=const(true)},
        buildings={isActive=guarded(function(z) return z.active end),
            containsTile=guarded(function(_, x, y) return y == 6 and (x == 5 or x == 7 or x == 8) end),
            markedForRemoval=guarded(function(b) return #b.jobs > 0 end)},
        maps={isTileVisible=guarded(function(x) return x ~= 8 end)},
    }
end
local pass='assigned_office_with_completed_chair_observed'
local unconfirmed='assigned_office_with_completed_chair_unconfirmed'
local function run(expect)
    script()
    assert(report.outcome==expect, report.outcome)
end
reset(); run(pass)
assert(report.site_id==17 and report.group_id==4 and report.year_tick==2000 and report.paused)
assert(report.manager_cooldown==733 and report.manager_timer==13)
assert(report.manager.assignment_id==11 and report.manager.historical_figure_id==361)
assert(report.manager.unit_id==256 and report.manager.current_job.type_key=='ManageWorkOrders')
assert(report.manager.manages_production and report.manager.required_office==1)
assert(report.offices[1].id==80 and report.offices[1].visible_chairs[1].id==81)

for _, modify in ipairs({
    function() unit.owned_buildings={} end,
    function() unit.owned_buildings={office,office} end,
    function() office.active=false end,
    function() office.flags.almost_deleted=true end,
    function() chair.stage=0 end,
    function() chair.flags.exists=false end,
    function() chair.jobs={{}} end,
    function() chair.flags.almost_deleted=true end,
    function() office.contained_buildings={} end,
    function() chair.centerx=6 end,  -- hole inside the rectangular footprint
    function() chair.z=3 end,
    function() chair.centerx=8 end,
}) do
    reset(); modify(); run(unconfirmed)
end
assert(report.offices[1].visibility_limited and #report.offices[1].visible_chairs==0)
reset(); office.assigned_unit_id=99; run('no_directly_assigned_office')
reset(); profile.histfig=-1; run('manager_position_vacant')
reset(); roles={}; run('manager_position_link_unresolved')
reset(); roles[2]=roles[1]; run('manager_position_link_unresolved')
reset(); unit.hist_figure_id=999; run('manager_identity_unresolved')
reset(); group.positions.own={}; run('no_manager_position')
reset(); group.positions.assignments={}; run('no_manager_assignment_record')

reset()
local extra=make_office(10)
extra.contained_buildings={chair}; unit.owned_buildings[2]=extra; other.ZONE_OFFICE[2]=extra
other.CHAIR[2]=make_chair(20,7,6,2)
run(pass)
assert(report.offices[1].id==10 and report.offices[2].id==80)
local seats=report.offices[1].visible_chairs
assert(seats[1].id==20 and seats[1].inside_zone and not seats[1].linked_to_zone)

for _, modify in ipairs({
    function() group.positions.assignments[2]=profile end,
    function() group.positions.own[2]=position end,
    function() office.room.extents=nil end,
    function() office.contained_buildings[2]=chair end,
    function() other.CHAIR={} end,
    function() other.CHAIR[2]=chair end,
    function() other.ZONE_OFFICE[2]=office end,
    function() for i=1,8193 do other.CHAIR[i]=chair end end,
    function() office.type=99 end,
    function() paused=false end,
    function() xml='wrong structures' end,
}) do
    reset(); modify()
    assert(not pcall(script) and report==nil and not suspended)
end
reset(); assert(not pcall(script,'unrecognized'))
