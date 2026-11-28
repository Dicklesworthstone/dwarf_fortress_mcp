-- Read the fortress Manager, assigned office zones, and installed chairs at one paused tick.
-- Usage: dfmcp-manager-office [--workshop=ID] [ORDER_ID ...]
local json = require('json')
local order_ids, requested, workshop_id = {}, {}, nil
local function native_id(arg)
    if not arg:match('^%d+$') or #arg > 10 or tonumber(arg) > 2147483647 then
        qerror('expected a nonnegative native ID')
    end
    return tonumber(arg)
end
for _, arg in ipairs({...}) do
    local workshop = arg:match('^%-%-workshop=(.*)$')
    if workshop then
        if workshop_id then qerror('select at most one workshop') end
        workshop_id = native_id(workshop)
    else
        local id = native_id(arg)
        if requested[id] or #order_ids == 8 then qerror('select at most eight distinct work orders') end
        requested[id] = true
        table.insert(order_ids, id)
    end
end
table.sort(order_ids)

local function bounded(vector, maximum, name)
    if #vector > maximum then qerror(name..' exceeds observation limit') end
    return vector
end

local function manager_at(group, report)
    local position
    for _, p in ipairs(bounded(group.positions.own, 1024, 'fortress positions')) do
        if p.code == 'MANAGER' then
            if position then qerror('ambiguous fortress Manager position') end
            position = p
        end
    end
    if not position then report.outcome = 'no_manager_position'; return end
    local manager = {position_id=position.id, required_office=position.required_office,
        manages_production=position.responsibilities[df.entity_position_responsibility.MANAGE_PRODUCTION]}
    report.manager = manager
    local assignment
    for _, a in ipairs(bounded(group.positions.assignments, 4096, 'fortress assignments')) do
        if a.position_id == position.id then
            if assignment then qerror('ambiguous fortress Manager assignment') end
            assignment = a
        end
    end
    if not assignment then report.outcome = 'no_manager_assignment_record'; return end
    manager.assignment_id, manager.historical_figure_id = assignment.id, assignment.histfig
    if assignment.histfig == -1 then report.outcome = 'manager_position_vacant'; return end
    local hf = df.historical_figure.find(assignment.histfig)
    local unit = hf and df.unit.find(hf.unit_id)
    if not unit or unit.hist_figure_id ~= assignment.histfig or hf.unit_id ~= unit.id then
        report.outcome = 'manager_identity_unresolved'; return
    end
    manager.unit_id = unit.id
    bounded(hf.entity_links, 1024, 'Manager historical links')
    local matching = 0
    for _, r in ipairs(dfhack.units.getNoblePositions(unit) or {}) do
        if r.entity.id == group.id and r.position.id == position.id and r.assignment.id == assignment.id then
            matching = matching + 1
        end
    end
    if matching ~= 1 then report.outcome = 'manager_position_link_unresolved'; return end
    manager.active = dfhack.units.isActive(unit)
    manager.citizen = dfhack.units.isCitizen(unit, true)
    local job = unit.job.current_job
    if job then
        manager.current_job = {id=job.id, job_type=job.job_type, type_key=df.job_type[job.job_type]}
    end
    return unit
end

local function by_id(a, b) return a.id < b.id end

local function offices_at(unit, report)
    local other = df.global.world.buildings.other
    local owned = {}
    for _, b in ipairs(bounded(unit.owned_buildings, 1024, 'owned buildings')) do
        owned[b.id] = (owned[b.id] or 0) + 1
    end
    local offices, seen = {}, {}
    for _, zone in ipairs(bounded(other.ZONE_OFFICE, 2048, 'office zones')) do
        if zone.type ~= df.civzone_type.Office or seen[zone.id] then qerror('invalid office zone index') end
        seen[zone.id] = true
        -- getOwner() may update owner_unit_cached_index. Read the owner ID directly.
        if zone.assigned_unit_id == unit.id then
            if #offices >= 32 then qerror('assigned offices exceed observation limit') end
            if not zone:isExtentShaped() or df.isnull(zone.room.extents) then qerror('missing office extents') end
            local row = {id=zone.id, active=dfhack.buildings.isActive(zone),
                owner_unit_id=zone.assigned_unit_id, reciprocal_owner_links=owned[zone.id] or 0,
                being_deleted=zone.flags.almost_deleted, visible_chairs={}, visibility_limited=false}
            local linked = {}
            for _, b in ipairs(bounded(zone.contained_buildings, 1024, 'zone buildings')) do
                if b:getType() == df.building_type.Chair then
                    if linked[b.id] then qerror('duplicate zone chair link') end
                    linked[b.id] = true
                end
            end
            offices[#offices+1] = {zone=zone, row=row, linked=linked}
        end
    end
    if #offices == 0 then report.outcome = 'no_directly_assigned_office'; return end
    local chairs = bounded(other.CHAIR, 8192, 'chair buildings')
    if #offices * #chairs > 65536 then qerror('office tile checks exceed observation limit') end
    local examined, rows, observed = {}, 0, false
    for _, chair in ipairs(chairs) do
        if chair:getType() ~= df.building_type.Chair or examined[chair.id] then qerror('invalid chair index') end
        examined[chair.id] = true
        for _, o in ipairs(offices) do
            local inside = chair.z == o.zone.z and dfhack.buildings.containsTile(o.zone, chair.centerx, chair.centery)
            local linked = o.linked[chair.id] == true
            if inside or linked then
                if dfhack.maps.isTileVisible(chair.centerx, chair.centery, chair.z) then
                    rows = rows + 1
                    if rows > 256 then qerror('visible chair rows exceed observation limit') end
                    bounded(chair.jobs, 256, 'chair jobs')
                    local stage, maximum = chair:getBuildStage(), chair:getMaxBuildStage()
                    local complete = chair.flags.exists and maximum > 0 and stage == maximum
                    local removing = chair.flags.almost_deleted or dfhack.buildings.markedForRemoval(chair)
                    local row = {id=chair.id, pos={chair.centerx, chair.centery, chair.z},
                        inside_zone=inside, linked_to_zone=linked, build_stage=stage, max_build_stage=maximum,
                        complete=complete, being_removed=removing}
                    table.insert(o.row.visible_chairs, row)
                    if inside and linked and complete and not removing and o.row.active
                            and not o.row.being_deleted and o.row.reciprocal_owner_links == 1 then
                        observed = true
                    end
                else
                    o.row.visibility_limited = true
                end
                o.linked[chair.id] = nil
            end
        end
    end
    for _, o in ipairs(offices) do
        if next(o.linked) then qerror('zone chair link absent from native chair index') end
        table.sort(o.row.visible_chairs, by_id)
        table.insert(report.offices, o.row)
    end
    table.sort(report.offices, by_id)
    report.outcome = observed and 'assigned_office_with_completed_chair_observed'
            or 'assigned_office_with_completed_chair_unconfirmed'
end

-- Work-order IDs, the owning building, and queue membership are separate native facts.
local function capture_production(report)
    local world = df.global.world
    local orders = {}
    for _, id in ipairs(order_ids) do orders[id] = {id=id, present=false} end
    if #order_ids > 0 then
        for _, o in ipairs(bounded(world.manager_orders.all, 8192, 'work orders')) do
            local row = orders[o.id]
            if row then
                if row.present then qerror('duplicate selected work order ID') end
                row.present, row.job_type, row.type_key = true, o.job_type, df.job_type[o.job_type]
                row.validated, row.active = o.status.validated, o.status.active
                row.amount_left, row.amount_total = o.amount_left, o.amount_total
                row.frequency, row.workshop_id, row.max_workshops = o.frequency, o.workshop_id, o.max_workshops
                row.finished_year, row.finished_year_tick = o.finished_year, o.finished_year_tick
                row.item_type, row.item_subtype = o.item_type, o.item_subtype
                row.material, row.material_category = {o.mat_type, o.mat_index}, o.material_category.whole
                row.requires_wood_log, row.has_item_requirements = o.material_category.wood, not df.isnull(o.items)
                row.item_conditions, row.order_conditions = #o.item_conditions, #o.order_conditions
            end
        end
    end
    local production = {orders={}, jobs={}}
    report.production = production
    for _, id in ipairs(order_ids) do table.insert(production.orders, orders[id]) end
    local queued, queue_position = {}, {}
    if workshop_id then
        local b = df.building.find(workshop_id)
        if not b or b:getType() ~= df.building_type.Workshop
                or not dfhack.maps.isTileVisible(b.centerx, b.centery, b.z) then
            qerror('selected visible workshop unavailable')
        end
        local queue = bounded(b.jobs, 256, 'workshop queue')
        production.workshop = {id=b.id, type_key=df.workshop_type[b:getSubtype()],
            build_stage=b:getBuildStage(), max_build_stage=b:getMaxBuildStage(), queue_size=#queue}
        local index = 0
        for _, job in ipairs(queue) do
            if queued[job.id] then qerror('duplicate workshop job ID') end
            queued[job.id], queue_position[job.id] = job, index
            index = index + 1
        end
    end
    local function add_job(job, in_world)
        if #production.jobs >= 256 then qerror('selected jobs exceed observation limit') end
        local row = {id=job.id, job_type=job.job_type, type_key=df.job_type[job.job_type],
            order_id=job.order_id, listed_in_world=in_world, selected_workshop_queue_index=queue_position[job.id],
            suspended=job.flags.suspend, working=job.flags.working, repeating=job.flags['repeat']}
        -- Read the native reference IDs without invoking cache-updating unit/building resolvers.
        for _, ref in ipairs(bounded(job.general_refs, 128, 'selected job references')) do
            local t = ref:getType()
            if t == df.general_ref_type.BUILDING_HOLDER then
                if row.holder_building_id then qerror('ambiguous job building holder') end
                row.holder_building_id = ref.building_id
            elseif t == df.general_ref_type.UNIT_WORKER then
                if row.worker_unit_id then qerror('ambiguous job worker') end
                row.worker_unit_id = ref.unit_id
            end
        end
        table.insert(production.jobs, row)
    end
    local seen, visited = {}, 0
    local link = world.jobs.list.next
    while link do
        visited = visited + 1
        if visited > 16384 then qerror('world job list exceeds observation limit') end
        local job = link.item
        if not job or seen[job.id] then qerror('invalid world job list') end
        seen[job.id] = true
        if requested[job.order_id] or queued[job.id] then
            if queued[job.id] and queued[job.id] ~= job then qerror('conflicting native job ID') end
            add_job(job, true)
        end
        link = link.next
    end
    for id, job in pairs(queued) do
        if not seen[id] then add_job(job, false) end
    end
    table.sort(production.jobs, by_id)
end

local function observe()
    local world = dfhack.world
    if dfhack.getDFHackVersion() ~= '53.16-r1.1'
            or dfhack.getGitXmlCommit() ~= '1dd01aad64219afa0578f1328cf15bd0c6006d5a' then
        qerror('expected DFHack 53.16-r1.1 with its pinned structures')
    end
    if not dfhack.isWorldLoaded() or not dfhack.isMapLoaded()
            or not world.isFortressMode() or not world.ReadPauseState() then
        qerror('a loaded paused fortress is required')
    end
    local plot = df.global.plotinfo
    local group = df.historical_entity.find(plot.group_id)
    if not group or group.id ~= plot.group_id then qerror('fortress entity unavailable') end
    local report = {schema='dfmcp.manager_office_observation/1',
        df_version=dfhack.getDFVersion(), dfhack_version=dfhack.getDFHackVersion(),
        world_folder=world.ReadWorldFolder(), site_id=world.GetCurrentSiteId(), group_id=group.id,
        paused=true, year=world.ReadCurrentYear(), year_tick=world.ReadCurrentTick(), offices={},
        manager_cooldown=plot.nobles.manager_cooldown, manager_timer=plot.manager_timer}
    local unit = manager_at(group, report)
    if unit then offices_at(unit, report) end
    if workshop_id or #order_ids > 0 then capture_production(report) end
    return report
end

print(json.encode(dfhack.with_suspend(observe), {pretty=false}))
