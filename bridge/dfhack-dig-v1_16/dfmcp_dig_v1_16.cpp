#include "../common/dig_designation_v1_16.h"
#include <array>
#include <cstdlib>
#include <memory>
#include <set>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "TileTypes.h"
#include "modules/Maps.h"
#include "modules/World.h"
#include "df/block_square_event_designation_priorityst.h"
#include "df/global_objects.h"
#include "df/job.h"
#include "df/job_list_link.h"
#include "df/map_block.h"
#include "df/tile_designation.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_material.h"
#include "df/tiletype_shape.h"
#include "df/world.h"
#include "DfmcpDigV1_16.pb.h"

using namespace DFHack;
namespace wire = dfmcp::dig::v1_16;
namespace dg = dfmcp_dig_v1_16;
DFHACK_PLUGIN("dfmcp_dig_v1_16");
namespace {
dg::Engine engine(static_cast<std::uint64_t>(dg::Clock::now().time_since_epoch().count()) | 1);
constexpr std::size_t MAX_JOBS = 65536, MAX_EVENTS = 4096;
constexpr std::uint16_t MAX_OBSERVED_TEMPERATURE = 10080;
bool enabled(const char *name) {
    const char *value = std::getenv(name); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(16); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in,out,3);
    dg::require(in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty());
    dg::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    dg::require(in->protocol_major() == 1 && in->protocol_minor() == 16,2);
    dg::require(enabled("DFMCP_ALLOW_UNADMITTED_DIG_V1_16"),1);
    const char *configured = std::getenv("DFMCP_DIG_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &provided = in->bearer_token();
    dg::require(expected.size() >= 32 && expected.size() <= 256 && provided.size() >= 32 && provided.size() <= 256,1);
    std::size_t different = expected.size() ^ provided.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < provided.size() ? provided[i] : 0;
        different |= a ^ b;
    }
    dg::require(different == 0,1); dg::require(engine.available(),5);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const char *version = Version::dfhack_version(); dg::require(version != nullptr,5);
    const std::string dfhack(version); dg::require(dg::utf8(df,128) && dg::utf8(dfhack,128),5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void require_fortress() {
    dg::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world && Maps::IsValid(),4);
}
df::block_square_event_designation_priorityst *priority_event(df::map_block *block) {
    dg::require(block && block->block_events.size() <= MAX_EVENTS,5);
    for (const auto *event : block->block_events) dg::require(event != nullptr,5);
    std::vector<df::block_square_event_designation_priorityst *> events;
    dg::require(Maps::SortBlockEvents(block,nullptr,nullptr,nullptr,nullptr,nullptr,nullptr,nullptr,&events),5);
    dg::require(events.size() <= 1,5); return events.empty() ? nullptr : events[0];
}
// Complete bounded scan is required even after a conflicting job is found.
// No worker is detached and no existing job is removed or rescheduled.
std::set<std::tuple<std::uint32_t,std::uint32_t,std::uint32_t>> jobs_in(const dg::Region &r) {
    std::set<std::int32_t> seen;
    std::set<std::tuple<std::uint32_t,std::uint32_t,std::uint32_t>> busy;
    auto *link = df::global::world->jobs.list.next;
    while (link) {
        dg::require(seen.size() < MAX_JOBS && link->item && link->item->id >= 0,5);
        const auto *job = link->item; dg::require(seen.insert(job->id).second,5);
        if (job->pos.x >= 0 && job->pos.y >= 0 && job->pos.z >= 0
            && r.target(job->pos.x,job->pos.y,job->pos.z)) busy.emplace(job->pos.x,job->pos.y,job->pos.z);
        link = link->next;
    }
    return busy;
}
dg::Observation capture(const dg::Region &r) {
    r.validate(); require_fortress();
    std::int32_t sx=0,sy=0,sz=0; Maps::getTileSize(sx,sy,sz);
    dg::require(sx > 0 && sy > 0 && sz > 0 && sx <= 32768 && sy <= 32768 && sz <= 32768
        && r.x+r.width < static_cast<std::uint32_t>(sx) && r.y+r.height < static_cast<std::uint32_t>(sy)
        && r.z+1 < static_cast<std::uint32_t>(sz),4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    dg::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX,5);
    dg::Observation out; out.region = r; out.tick = static_cast<std::uint64_t>(year)*403200+tick;
    out.site = static_cast<std::uint32_t>(site); out.folder = World::ReadWorldFolder(); out.paused = World::ReadPauseState();
    out.size_x = sx; out.size_y = sy; out.size_z = sz; out.tiles.reserve(r.cells());
    const auto jobs = jobs_in(r);
    std::map<df::map_block *,df::block_square_event_designation_priorityst *> priorities;
    for (auto z=r.z-1; z<=r.z+1; ++z)
    for (auto y=r.y-1; y<=r.y+r.height; ++y)
    for (auto x=r.x-1; x<=r.x+r.width; ++x) {
        dg::Cell cell;
        auto *block = Maps::getTileBlock(static_cast<std::int32_t>(x),static_cast<std::int32_t>(y),static_cast<std::int32_t>(z));
        if (!block) { out.tiles.push_back(cell); continue; }
        const auto lx=x&15,ly=y&15; auto des=block->designation[lx][ly];
        if (des.bits.hidden) { cell.presence=1; out.tiles.push_back(cell); continue; }
        // Do not examine hidden type/material/temperature/priority/occupancy.
        const auto tt=block->tiletype[lx][ly]; dg::require(static_cast<int>(tt)>=0 && is_valid_enum_item(tt),5);
        const auto &occ=block->occupancy[lx][ly];
        cell.presence=2; cell.tiletype=static_cast<std::uint32_t>(tt);
        cell.dig=static_cast<std::uint8_t>(des.bits.dig); des.bits.dig=df::tile_dig_designation::No;
        cell.designation_other=des.whole; cell.occupancy=occ.whole;
        cell.temperature1=block->temperature_1[lx][ly]; cell.temperature2=block->temperature_2[lx][ly];
        cell.smooth=des.bits.smooth != 0;
        cell.occupied=occ.bits.building || occ.bits.unit || occ.bits.unit_grounded || occ.bits.item;
        cell.job=jobs.count({x,y,z}) != 0;
        auto flags=block->flags; cell.designated=flags.bits.designated; flags.bits.designated=false;
        cell.block_other=flags.whole; cell.cooldown=static_cast<std::uint32_t>(block->dsgn_check_cooldown);
        const auto material=tileMaterial(tt);
        cell.natural_wall=tileShape(tt)==df::tiletype_shape::WALL &&
            (material==df::tiletype_material::STONE || material==df::tiletype_material::SOIL || material==df::tiletype_material::MINERAL);
        cell.hazards=static_cast<std::uint8_t>((des.bits.flow_size ? 1 : 0)
            | (Maps::isTileAquifer(x,y,z) ? 2 : 0) | ((des.bits.feature_local || des.bits.feature_global) ? 4 : 0)
            | ((cell.temperature1>MAX_OBSERVED_TEMPERATURE || cell.temperature2>MAX_OBSERVED_TEMPERATURE) ? 8 : 0));
        auto found=priorities.find(block);
        if (found==priorities.end()) {
            dg::require(priorities.size()<12,5); found=priorities.emplace(block,priority_event(block)).first;
        }
        if (found->second) cell.priority=found->second->priority[lx][ly];
        out.tiles.push_back(cell);
    }
    return out;
}
void designate(const dg::Region &r) {
    // Engine has just revalidated the complete selection+halo under the SAME
    // CoreSuspender. Preallocate every optional priority event and vector slot
    // before writing any persistent native field. At most four target blocks.
    r.validate(); require_fortress(); dg::require(World::ReadPauseState(),4);
    struct Block {
        df::map_block *block=nullptr;
        df::block_square_event_designation_priorityst *priority=nullptr;
        std::unique_ptr<df::block_square_event_designation_priorityst> fresh;
    };
    std::array<Block,4> blocks; std::size_t count=0;
    for (auto by=r.y>>4; by<=(r.y+r.height-1)>>4; ++by)
    for (auto bx=r.x>>4; bx<=(r.x+r.width-1)>>4; ++bx) {
        auto &entry=blocks[count++]; entry.block=Maps::getTileBlock(bx*16,by*16,r.z);
        dg::require(entry.block,5); entry.priority=priority_event(entry.block);
        if (!entry.priority) {
            dg::require(entry.block->block_events.size()<MAX_EVENTS,5);
            entry.fresh=std::make_unique<df::block_square_event_designation_priorityst>();
            for (unsigned x=0;x<16;++x) for (unsigned y=0;y<16;++y) entry.fresh->priority[x][y]=0;
            entry.priority=entry.fresh.get(); entry.block->block_events.reserve(entry.block->block_events.size()+1);
        }
    }
    for (std::size_t i=0;i<count;++i) {
        auto &entry=blocks[i];
        if (entry.fresh) { entry.block->block_events.push_back(entry.fresh.get()); (void)entry.fresh.release(); }
    }
    for (auto y=r.y;y<r.y+r.height;++y) for (auto x=r.x;x<r.x+r.width;++x) {
        auto *block=Maps::getTileBlock(x,y,r.z); dg::require(block,5);
        df::block_square_event_designation_priorityst *priority=nullptr;
        for (std::size_t i=0;i<count;++i) if (blocks[i].block==block) priority=blocks[i].priority;
        dg::require(priority,5);
        priority->priority[x&15][y&15]=dg::PRIORITY;
        block->designation[x&15][y&15].bits.dig=df::tile_dig_designation::Default;
        block->flags.bits.designated=true; block->dsgn_check_cooldown=0;
    }
}
unsigned shape(const wire::Request *in) {
    return (in->has_x()?1:0)|(in->has_y()?2:0)|(in->has_z()?4:0)|(in->has_width()?8:0)|(in->has_height()?16:0)
        |(in->has_allow_hidden_neighbors()?32:0)|(in->has_idempotency_key()?64:0)|(in->has_expected_witness()?128:0)
        |(in->has_plan_digest()?256:0)|(in->has_prepare_token()?512:0);
}
dg::Region region(const wire::Request *in) { dg::Region r{in->x(),in->y(),in->z(),in->width(),in->height()}; r.validate(); return r; }
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out, Body body) {
    try { authorize(in,out); body(); out->set_failure_code(0); out->set_accepted(true); return CR_OK; }
    catch (const dg::Failure &e) { try { empty_reply(in,out,e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; } }
    catch (...) { try { empty_reply(in,out,5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; } }
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==0);});
}
command_result ReadDesignation(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==31);out->set_observation(engine.inspect(region(in),capture).encode());});
}
command_result PrepareDesignation(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==511);dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);
        const auto prepared=engine.prepare(in->idempotency_key(),region(in),in->allow_hidden_neighbors(),
            in->expected_witness(),in->plan_digest(),dg::Clock::now(),capture);
        out->set_effect_record(prepared.first->encode());out->set_replayed(prepared.second);});
}
command_result CommitDesignation(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==832);dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);
        out->set_effect_record(engine.commit(in->idempotency_key(),in->plan_digest(),in->prepare_token(),dg::Clock::now(),capture,designate).encode());});
}
command_result QueryDesignation(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==320);
        if(const auto *r=engine.query(in->idempotency_key(),in->plan_digest()))out->set_effect_record(r->encode());});
}
command_result CancelDesignation(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==832);dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);
        out->set_effect_record(engine.cancel(in->idempotency_key(),in->plan_digest(),in->prepare_token()).encode());});
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { engine.reset(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event) {
    if(event==SC_MAP_LOADED||event==SC_MAP_UNLOADED||event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED)engine.reset();
    else if(event==SC_PAUSED||event==SC_UNPAUSED)engine.interrupt();
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service=new RPCService();
    service->addFunction("Handshake",Handshake,0);service->addFunction("ReadDesignation",ReadDesignation,0);
    service->addFunction("PrepareDesignation",PrepareDesignation,0);service->addFunction("CommitDesignation",CommitDesignation,0);
    service->addFunction("QueryDesignation",QueryDesignation,0);service->addFunction("CancelDesignation",CancelDesignation,0);
    return service;
}
