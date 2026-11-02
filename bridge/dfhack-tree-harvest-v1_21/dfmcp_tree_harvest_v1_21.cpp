#include "../common/tree_harvest.h"
#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <memory>
#include <set>
#include <string_view>
#include "Core.h"
#include "modules/Designations.h"
#include "df/itemdef_weaponst.h"
#include "df/job_skill.h"
#include "df/building_wagonst.h"
#include "df/plant.h"
#include "df/plant_tree_info.h"
#include "df/plant_tree_tile.h"
#include "df/tile_dig_designation.h"
#include "df/tiletype_material.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "TileTypes.h"
#include "modules/Buildings.h"
#include "modules/Job.h"
#include "modules/Maps.h"
#include "modules/Materials.h"
#include "modules/Units.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/building_civzonest.h"
#include "df/building_type.h"
#include "df/building_workshopst.h"
#include "df/buildingitemst.h"
#include "df/general_ref.h"
#include "df/general_ref_type.h"
#include "df/global_objects.h"
#include "df/item.h"
#include "df/item_type.h"
#include "df/job.h"
#include "df/job_item_ref.h"
#include "df/job_list_link.h"
#include "df/job_role_type.h"
#include "df/job_type.h"
#include "df/map_block.h"
#include "df/material.h"
#include "df/material_flags.h"
#include "df/specific_ref.h"
#include "df/specific_ref_type.h"
#include "df/tile_building_occ.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_shape.h"
#include "df/unit.h"
#include "df/unit_labor.h"
#include "df/workshop_type.h"
#include "df/world.h"
#include "DfmcpTreeHarvestV1_21.pb.h"

using namespace DFHack;
namespace wire = dfmcp::tree_harvest::v1_21;
namespace ws = dfmcp_workshop;
namespace th = dfmcp_tree_harvest;
DFHACK_PLUGIN("dfmcp_tree_harvest_v1_21");
namespace {
std::uint64_t monotonic_ms() {
    const auto value = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now().time_since_epoch()).count();
    ws::require(value >= 0, 5); return static_cast<std::uint64_t>(value);
}
th::Engine engine(monotonic_ms() | 1);
constexpr std::size_t MAX_REQUEST_BYTES = 2048, MAX_REPLY_BYTES = 512 * 1024;
constexpr std::size_t MAX_REFS = 4096, MAX_JOBS = 65536, MAX_ITEMS = 1048576, MAX_UNITS = 8192;
constexpr std::size_t MAX_BUILDING_JOBS = 8, MAX_CONTAINED = 64;
bool enabled(const char *name) {
    const char *value = std::getenv(name); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(21); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize_credential(const std::string &provided) {
    const char *configured = std::getenv("DFMCP_TREE_HARVEST_TOKEN");
    const std::string_view expected = configured ? configured : "";
    ws::require(expected.size() >= 32 && expected.size() <= 256
        && provided.size() >= 32 && provided.size() <= 256, 1);
    std::size_t difference = expected.size() ^ provided.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0;
        const unsigned char b = i < provided.size() ? provided[i] : 0;
        difference |= a ^ b;
    }
    ws::require(difference == 0, 1);
}
bool development_profile() {
    return std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") == nullptr
        && enabled("DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    ws::require(static_cast<std::size_t>(in->ByteSize()) <= MAX_REQUEST_BYTES && in->IsInitialized()
        && in->GetReflection()->GetUnknownFields(*in).empty());
    ws::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    ws::require(in->protocol_major() == 1 && in->protocol_minor() == 21, 2);
    ws::require(development_profile(), 1);
    authorize_credential(in->bearer_token());
}
void manifest(wire::Reply *out) {
    ws::require(engine.available(), 5);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const char *version = Version::dfhack_version(); ws::require(version != nullptr, 5);
    const std::string dfhack(version); ws::require(ws::utf8(df, 128) && ws::utf8(dfhack, 128), 5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void require_fortress() {
    ws::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world && Maps::IsValid(), 4);
}
df::coord native_pos(ws::Coord p) {
    (void)p.encode(); return df::coord(static_cast<std::int16_t>(p.x),
        static_cast<std::int16_t>(p.y), static_cast<std::int16_t>(p.z));
}
struct Source {
    std::uint64_t tick;
    std::uint32_t site;
    std::array<std::uint32_t,3> dimensions;
    std::string folder;
};
Source source() {
    require_fortress(); std::int32_t x=0,y=0,z=0; Maps::getTileSize(x,y,z);
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick=World::ReadCurrentTick(); const auto site=World::GetCurrentSiteId();
    th::require(x>0 && y>0 && z>0 && x<=32768 && y<=32768 && z<=32768
        && year>=0 && year<=UINT32_MAX && tick<403200 && site>=0
        && df::global::item_next_id && *df::global::item_next_id>=0,5);
    return {static_cast<std::uint64_t>(year)*403200+tick,static_cast<std::uint32_t>(site),
        {static_cast<std::uint32_t>(x),static_cast<std::uint32_t>(y),static_cast<std::uint32_t>(z)},World::ReadWorldFolder()};
}
bool visible(df::coord pos) {
    if(pos.x<0 || pos.y<0 || pos.z<0) return false;
    const auto *block=Maps::getTileBlock(pos);
    return block && !block->designation[pos.x&15][pos.y&15].bits.hidden;
}
th::Coord coord(df::coord p) {
    th::require(p.x>=0 && p.y>=0 && p.z>=0,5);
    return {static_cast<std::uint32_t>(p.x),static_cast<std::uint32_t>(p.y),static_cast<std::uint32_t>(p.z)};
}
df::plant *plant_at(th::Coord root) {
    const auto &all=df::global::world->plants.all;
    th::require(all.size()<=th::MAX_PLANTS,5); std::set<const df::plant*> seen;
    df::plant *found=nullptr;
    for(auto *p:all) {
        th::require(p && seen.insert(p).second,5);
        if(p->pos==native_pos(root)) { th::require(!found,5); found=p; }
    }
    return found;
}
std::vector<df::job*> jobs() {
    std::vector<df::job*> out; std::set<std::int32_t> seen;
    auto *previous=&df::global::world->jobs.list;
    for(auto *link=previous->next;link;link=link->next) {
        th::require(out.size()<th::MAX_JOBS && link->item && link->prev==previous
            && link->item->id>=0 && seen.insert(link->item->id).second && link->item->list_link==link,5);
        out.push_back(link->item); previous=link;
    }
    return out;
}
th::Cell cell(th::Coord p) {
    th::Cell out; out.pos=p;
    const auto *b=Maps::getTileBlock(native_pos(p)); if(!b) return out;
    const auto x=p.x&15,y=p.y&15; auto des=b->designation[x][y];
    if(des.bits.hidden) { out.presence=1; return out; }
    out.presence=2; out.tiletype=static_cast<std::uint32_t>(b->tiletype[x][y]);
    out.dig=static_cast<unsigned char>(des.bits.dig); des.bits.dig=df::tile_dig_designation::No;
    out.designation_other=des.whole; out.occupancy=b->occupancy[x][y].whole;
    auto flags=b->flags; out.designated=flags.bits.designated; flags.bits.designated=false;
    out.block_other=flags.whole; out.temperature1=b->temperature_1[x][y]; out.temperature2=b->temperature_2[x][y];
    out.liquid=static_cast<unsigned char>(b->designation[x][y].bits.flow_size); return out;
}
bool approach(df::coord from,th::Coord root) {
    for(int y=-1;y<=1;++y) for(int x=-1;x<=1;++x) {
        if(!x&&!y) continue;
        const auto px=static_cast<std::int32_t>(root.x)+x, py=static_cast<std::int32_t>(root.y)+y;
        if(px<0||py<0||px>=32768||py>=32768) continue;
        const df::coord to(px,py,root.z);
        if(visible(to)&&Maps::canWalkBetween(from,to)) return true;
    }
    return false;
}
std::vector<th::Worker> workers(th::Coord root,bool &complete) {
    const auto &active=df::global::world->units.active;
    th::require(active.size()<=th::MAX_UNITS,5); std::set<std::int32_t> seen;
    std::vector<th::Worker> out;
    for(auto *u:active) {
        th::require(u && u->id>=0 && seen.insert(u->id).second,5);
        if(!Units::isActive(u)||!Units::isAlive(u)||!Units::isCitizen(u,true)) continue;
        if(out.size()==32) { complete=false; continue; }
        th::Worker w; w.id=u->id; w.pos=coord(u->pos); w.visible=visible(u->pos);
        w.adult=Units::isAdult(u); w.sane=Units::isSane(u);
        w.labor=u->status.labors[df::unit_labor::CUTWOOD];
        w.labor_valid=Units::isValidLabor(u,df::unit_labor::CUTWOOD);
        w.job_available=Units::isJobAvailable(u,false);
        w.reachable=w.visible&&approach(u->pos,root); out.push_back(w);
    }
    std::sort(out.begin(),out.end(),[](const auto&a,const auto&b){return a.id<b.id;}); return out;
}
std::vector<th::Tool> tools(th::Coord root,bool &complete) {
    const auto &all=df::global::world->items.all; th::require(all.size()<=th::MAX_ITEMS,5);
    std::set<std::int32_t> ids; std::vector<th::Tool> out;
    for(auto *item:all) {
        th::require(item && item->id>=0 && ids.insert(item->id).second,5);
        if(item->flags.bits.hidden || !visible(item->pos)) continue;
        if(item->getType()!=df::item_type::WEAPON) continue;
        const auto *def=df::itemdef_weaponst::find(item->getSubtype());
        if(!def || def->skill_melee!=df::job_skill::AXE) continue;
        if(out.size()==th::MAX_TOOLS) {complete=false;continue;}
        th::Tool t; t.id=item->id; t.flags=item->flags.whole; t.flags2=item->flags2.whole;
        t.pos=coord(item->pos); t.axe=t.visible=true; t.reachable=approach(item->pos,root);
        if(item->general_refs.size()==1 && item->specific_refs.empty() && item->general_refs[0]
            && item->general_refs[0]->getType()==df::general_ref_type::BUILDING_HOLDER) {
            auto *holder=virtual_cast<df::building_wagonst>(item->general_refs[0]->getBuilding());
            if(holder && holder->id>=0 && holder->contained_items.size()<=64) {
                std::set<std::int32_t> contained; unsigned matches=0;
                for(const auto *entry:holder->contained_items) {
                    th::require(entry && entry->item && entry->item->id>=0
                        && contained.insert(entry->item->id).second,5);
                    if(entry->item==item) {++matches;t.role=static_cast<std::int32_t>(entry->use_mode);}
                }
                t.holder=holder->id; t.reciprocal=matches==1;
            }
        }
        out.push_back(t);
    }
    std::sort(out.begin(),out.end(),[](const auto&a,const auto&b){return a.id<b.id;}); return out;
}
th::Capture capture(const th::Selection &selection) {
    const auto current=source(); th::Capture out; out.selection=selection;
    out.tick=current.tick; out.site=current.site; out.dimensions=current.dimensions; out.folder=current.folder;
    out.next_item=*df::global::item_next_id; out.paused=World::ReadPauseState();
    const auto r=selection.root;
    th::require(r.x<out.dimensions[0]&&r.y<out.dimensions[1]&&r.z<out.dimensions[2],4);
    const auto root_cell=cell(r); out.root_presence=root_cell.presence;
    if(out.root_presence!=2) return out;
    auto *plant=plant_at(r); if(!plant || !plant->tree_info) return out;
    out.species=plant->material; out.alive=!plant->damage_flags.bits.dead&&!plant->damage_flags.bits.season_dead;
    auto *tree=plant->tree_info; out.mature=true;
    th::require(tree->dim_x>0 && tree->dim_y>0 && tree->body_height>0
        && tree->dim_x<=64 && tree->dim_y<=64 && tree->body_height<=64
        && static_cast<std::uint64_t>(tree->dim_x)*tree->dim_y*tree->body_height<=th::MAX_CELLS
        && tree->body,5);
    out.width=tree->dim_x; out.height=tree->dim_y; out.body_height=tree->body_height;
    const auto x1=static_cast<std::int32_t>(r.x)-tree->dim_x/2;
    const auto y1=static_cast<std::int32_t>(r.y)-tree->dim_y/2;
    th::require(x1>0 && y1>0 && x1+tree->dim_x<static_cast<std::int32_t>(out.dimensions[0])
        && y1+tree->dim_y<static_cast<std::int32_t>(out.dimensions[1])
        && r.z+out.body_height<out.dimensions[2],4);
    const auto volume=static_cast<std::uint64_t>(tree->dim_x+2)*(tree->dim_y+2)*tree->body_height;
    th::require(volume<=th::MAX_CELLS,5);
    // Check all terrain visibility before reading or serializing tree body tiles.
    for(std::uint32_t z=r.z;z<r.z+out.body_height;++z)
    for(int y=y1-1;y<=y1+tree->dim_y;++y) for(int x=x1-1;x<=x1+tree->dim_x;++x)
        out.cells.push_back(cell({static_cast<std::uint32_t>(x),static_cast<std::uint32_t>(y),z}));
    out.complete=true;
    for(const auto &c:out.cells) if(c.presence!=2) out.complete=false;
    if(!out.complete) return out;
    for(int z=0;z<tree->body_height;++z) {
        th::require(tree->body[z],5);
        for(int i=0;i<tree->dim_x*tree->dim_y;++i) out.body.push_back(tree->body[z][i].whole);
    }
    const auto *block=Maps::getTileBlock(native_pos(r));
    th::require(block && tileMaterial(block->tiletype[r.x&15][r.y&15])==df::tiletype_material::TREE
        && tileShape(block->tiletype[r.x&15][r.y&15])==df::tiletype_shape::WALL,4);
    out.designation=coord(Designations::getPlantDesignationTile(plant));
    for(const auto *job:jobs()) if(job->pos==native_pos(out.designation)) out.existing_job=true;
    out.workers=workers(out.designation,out.complete); out.tools=tools(out.designation,out.complete); return out;
}
void designate(const th::Capture &before,const std::string &credential) {
    // The engine has advanced its sequence; no game write has occurred yet.
    auto exact=engine.inspect(before.selection,capture); exact.sequence=before.sequence;
    th::require(exact.encode()==before.encode() && exact.eligible(),6);
    th::require(development_profile() && enabled("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE"),1);
    authorize_credential(credential);
    auto *plant=plant_at(before.selection.root); th::require(plant,6);
    (void)Designations::markPlant(plant);
}
th::Insertion verify(const th::Capture &before) {
    auto *plant=plant_at(before.selection.root); th::require(plant,5);
    th::Insertion p; p.root=coord(plant->pos); p.designation=coord(Designations::getPlantDesignationTile(plant));
    p.species=plant->material; p.marked=Designations::isPlantMarked(plant); return p;
}
unsigned shape(const wire::Request *in) {
    return (in->has_x()?1:0)|(in->has_y()?2:0)|(in->has_z()?4:0)
        |(in->has_idempotency_key()?8:0)|(in->has_expected_witness()?16:0)
        |(in->has_plan_digest()?32:0)|(in->has_prepare_token()?64:0)
        |(in->has_since_item_id()?128:0)|(in->has_species()?256:0);
}
th::Selection selection(const wire::Request *in) { return {{in->x(),in->y(),in->z()}}; }
template<class Body> command_result guarded(const wire::Request *in,wire::Reply *out,unsigned mask,bool write,Body body) {
    try {
        authorize(in,out); th::require(shape(in)==mask);
        if(write) th::require(enabled("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE"),1);
        manifest(out); body(); out->set_unresolved(engine.unresolved()); out->set_retained_records(engine.size());
        out->set_failure_code(0); out->set_accepted(true);
        th::require(static_cast<std::size_t>(out->ByteSize())<=MAX_REPLY_BYTES,5); return CR_OK;
    } catch(const ws::Failure &e) { empty_reply(in,out,e.code); return CR_OK; }
    catch(...) { empty_reply(in,out,5); return CR_OK; }
}
command_result Handshake(color_ostream&,const wire::Request*in,wire::Reply*out) {return guarded(in,out,0,false,[]{});}
command_result ReadTreeHarvest(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,7,false,[&]{out->set_observation(engine.inspect(selection(in),capture).encode());});
}
command_result PrepareTreeHarvest(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,63,true,[&]{
        const bool replay=engine.query(in->idempotency_key(),in->plan_digest())!=nullptr;
        out->set_effect_record(engine.prepare(in->idempotency_key(),selection(in),in->expected_witness(),
            in->plan_digest(),monotonic_ms(),capture).encode()); out->set_replayed(replay);
    });
}
command_result CommitTreeHarvest(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,104,true,[&]{out->set_effect_record(engine.commit(in->idempotency_key(),in->plan_digest(),
        in->prepare_token(),monotonic_ms(),capture,[&](const th::Capture&b){designate(b,in->bearer_token());},verify).encode());});
}
command_result QueryTreeHarvest(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,40,false,[&]{if(const auto*r=engine.reconcile(in->idempotency_key(),in->plan_digest(),capture,verify))
        out->set_effect_record(r->encode());});
}
command_result CancelTreeHarvest(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,104,false,[&]{out->set_effect_record(engine.cancel(in->idempotency_key(),in->plan_digest(),in->prepare_token()).encode());});
}
std::string progress(const wire::Request *in) {
    const auto s=source(); const auto root=selection(in).root;
    th::require(root.x<s.dimensions[0]&&root.y<s.dimensions[1]&&root.z<s.dimensions[2]
        && in->since_item_id()<=static_cast<std::uint32_t>(*df::global::item_next_id),4);
    std::string out="DFMTP021"; th::u64(out,engine.generation()); th::u64(out,engine.sequence()); th::u64(out,s.tick);
    th::u32(out,s.site); for(auto n:s.dimensions) th::u32(out,n); th::field(out,s.folder); out+=root.encode();
    th::flag(out,World::ReadPauseState()); unsigned char predicate=0;
    if(visible(native_pos(root))) {
        auto *p=plant_at(root);
        predicate=(!p||!p->tree_info)?2:(p->material==in->species()?1:3);
    }
    out.push_back(static_cast<char>(predicate)); th::i32(out,in->species()); th::u32(out,*df::global::item_next_id);
    unsigned count=0; for(const auto *j:jobs()) if(j->job_type==df::job_type::FellTree && j->pos.z==static_cast<int>(root.z)
        && std::abs(j->pos.x-static_cast<int>(root.x))<=32 && std::abs(j->pos.y-static_cast<int>(root.y))<=32) ++count;
    th::require(count<=65535,5); th::u16(out,static_cast<std::uint16_t>(count));
    std::vector<std::pair<std::uint32_t,std::string>> rows; std::set<std::int32_t> ids;
    const auto &all=df::global::world->items.all; th::require(all.size()<=th::MAX_ITEMS,5);
    for(auto *item:all) {
        th::require(item&&item->id>=0&&ids.insert(item->id).second,5);
        if(static_cast<std::uint32_t>(item->id)<in->since_item_id() || item->flags.bits.hidden
            || !item->flags.bits.on_ground || item->flags.bits.in_job || item->flags2.whole || !visible(item->pos)) continue;
        auto flags=item->flags; flags.bits.on_ground=false;
        if((flags.whole&~ws::COMPUTED_ITEM_FLAGS)||!item->general_refs.empty()||!item->specific_refs.empty()
            ||item->getType()!=df::item_type::WOOD||item->getWear()!=0) continue;
        MaterialInfo material(item->getMaterial(),item->getMaterialIndex());
        if(!material.isPlant()||!material.material||!material.material->flags.is_set(df::material_flags::WOOD)) continue;
        const auto *block=Maps::getTileBlock(item->pos); const auto x=item->pos.x&15,y=item->pos.y&15;
        if(tileShape(block->tiletype[x][y])!=df::tiletype_shape::FLOOR || block->designation[x][y].bits.flow_size
            || block->occupancy[x][y].bits.building!=df::tile_building_occ::None) continue;
        th::require(rows.size()<64,5);std::string row;
        th::u32(row,item->id); row+=coord(item->pos).encode();
        th::i32(row,item->getMaterial()); th::i32(row,item->getMaterialIndex()); th::u32(row,item->flags.whole);
        rows.emplace_back(static_cast<std::uint32_t>(item->id),std::move(row));
    }
    // New eligible supply observations do not establish tree provenance.
    std::sort(rows.begin(),rows.end(),[](const auto&a,const auto&b){return a.first<b.first;});
    th::u16(out,static_cast<std::uint16_t>(rows.size()));for(const auto &row:rows) out+=row.second;
    return out;
}
command_result ReadTreeHarvestProgress(color_ostream&,const wire::Request*in,wire::Reply*out) {
    return guarded(in,out,391,false,[&]{out->set_observation(progress(in));});
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream&,std::vector<PluginCommand>&) {return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream&) {engine.change_source();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream&,state_change_event event) {
    if(event==SC_MAP_LOADED||event==SC_MAP_UNLOADED||event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED) engine.change_source();
    else if(event==SC_PAUSED||event==SC_UNPAUSED) engine.interrupt();
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream&) {
    auto *service=new RPCService();
    service->addFunction("Handshake",Handshake,0); service->addFunction("ReadTreeHarvest",ReadTreeHarvest,0);
    service->addFunction("PrepareTreeHarvest",PrepareTreeHarvest,0); service->addFunction("CommitTreeHarvest",CommitTreeHarvest,0);
    service->addFunction("QueryTreeHarvest",QueryTreeHarvest,0); service->addFunction("CancelTreeHarvest",CancelTreeHarvest,0);
    service->addFunction("ReadTreeHarvestProgress",ReadTreeHarvestProgress,0); return service;
}
