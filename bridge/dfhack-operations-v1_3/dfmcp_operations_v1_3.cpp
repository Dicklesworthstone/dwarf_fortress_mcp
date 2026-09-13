#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <map>
#include <string>
#include <string_view>
#include <tuple>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "MiscUtils.h"
#include "DfmcpOperationsV1_3.pb.h"
#include "modules/Job.h"
#include "modules/Items.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/building_type.h"
#include "df/global_objects.h"
#include "df/item.h"
#include "df/item_type.h"
#include "df/job.h"
#include "df/job_item_ref.h"
#include "df/job_list_link.h"
#include "df/unit.h"
#include "df/world.h"

using namespace DFHack;
namespace wire = dfmcp::operations::v1_3;
DFHACK_PLUGIN("dfmcp_operations_v1_3");

namespace {
constexpr std::size_t MAX_JOBS=4096, MAX_BUILDINGS=4096, MAX_ITEMS=32768;
constexpr std::size_t MAX_ATTACHMENTS=65536, MAX_PAYLOAD=2*1024*1024;
std::uint64_t generation=static_cast<std::uint64_t>(
    std::chrono::steady_clock::now().time_since_epoch().count()) | std::uint64_t{1};

bool valid_utf8(const std::string &value, std::size_t maximum, bool empty)
{
    if (value.size()>maximum || (!empty && value.empty())) return false;
    for (std::size_t i=0; i<value.size();) {
        const auto c=static_cast<unsigned char>(value[i]);
        if (!c) return false;
        if (c<0x80) { ++i; continue; }
        const std::size_t n=c>=0xC2 && c<=0xDF ? 2 : c>=0xE0 && c<=0xEF ? 3 : c>=0xF0 && c<=0xF4 ? 4 : 0;
        if (!n || i+n>value.size()) return false;
        for (std::size_t j=1; j<n; ++j)
            if ((static_cast<unsigned char>(value[i+j]) & 0xC0)!=0x80) return false;
        const auto second=static_cast<unsigned char>(value[i+1]);
        if ((c==0xE0 && second<0xA0) || (c==0xED && second>=0xA0) ||
            (c==0xF0 && second<0x90) || (c==0xF4 && second>=0x90)) return false;
        i+=n;
    }
    return true;
}
void u32(std::string &out, std::uint32_t value)
{
    for (int shift=24; shift>=0; shift-=8) out.push_back(static_cast<char>((value>>shift)&255));
}
void i32(std::string &out, std::int32_t value) { u32(out,static_cast<std::uint32_t>(value)); }
void text(std::string &out, const std::string &value)
{
    out.push_back(static_cast<char>((value.size()>>8)&255));
    out.push_back(static_cast<char>(value.size()&255)); out.append(value);
}
void reference(std::string &out, std::int32_t value)
{
    out.push_back(value>=0 ? 1 : 0); if (value>=0) u32(out,static_cast<std::uint32_t>(value));
}
bool authorize(const wire::Request *in, wire::Reply *out)
{
    out->Clear(); out->set_accepted(false); out->set_failure_code(3);
    out->set_client_nonce(""); out->set_protocol_major(1); out->set_protocol_minor(3);
    out->set_bridge_generation(0); out->set_df_version(""); out->set_dfhack_version("");
    if (in->client_nonce().size()<16 || in->client_nonce().size()>64 ||
        !in->max_jobs() || in->max_jobs()>MAX_JOBS || !in->max_buildings() || in->max_buildings()>MAX_BUILDINGS ||
        !in->max_items() || in->max_items()>MAX_ITEMS || in->max_bytes()<1024 || in->max_bytes()>MAX_PAYLOAD) return false;
    out->set_client_nonce(in->client_nonce());
    if (in->protocol_major()!=1 || in->protocol_minor()!=3) { out->set_failure_code(2); return false; }
    const char *configured=std::getenv("DFMCP_OPERATIONS_TOKEN");
    const std::string_view expected=configured ? std::string_view(configured) : std::string_view();
    const auto &presented=in->bearer_token();
    out->set_failure_code(1);
    if (expected.size()<32 || expected.size()>256 || presented.size()<32 || presented.size()>256) return false;
    std::size_t difference=expected.size() ^ presented.size();
    for (std::size_t i=0; i<256; ++i) {
        const unsigned char a=i<expected.size() ? expected[i] : 0;
        const unsigned char b=i<presented.size() ? presented[i] : 0;
        difference|=static_cast<std::size_t>(a^b);
    }
    if (difference) return false;
    out->set_failure_code(5);
    const auto &v=Core::getInstance().vinfo;
    const std::string df=v ? v->getVersion() : std::string();
    const std::string dfhack=Version::dfhack_version();
    if (!generation || generation==std::numeric_limits<std::uint64_t>::max() ||
        !valid_utf8(df,128,false) || !valid_utf8(dfhack,128,false)) return false;
    out->set_bridge_generation(generation); out->set_df_version(df); out->set_dfhack_version(dfhack);
    out->set_failure_code(0); return true;
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out)
{
    if (authorize(in,out)) out->set_accepted(true);
    return CR_OK;
}

template<class T>
bool roster(const std::vector<T *> &source, std::size_t maximum, std::int32_t horizon,
            std::vector<T *> &ordered, std::map<std::int32_t,T *> &by_id)
{
    if (source.size()>maximum || horizon<0) return false;
    for (auto *value:source) {
        if (!value || value->id<0 || value->id>=horizon || !by_id.emplace(value->id,value).second) return false;
    }
    for (const auto &entry:by_id) ordered.push_back(entry.second);
    return true;
}
template<class T> bool member(const std::map<std::int32_t,T *> &roster, T *value)
{
    if (!value) return true;
    const auto found=roster.find(value->id);
    return found!=roster.end() && found->second==value;
}
struct Attachment {
    std::int32_t job,item,role,filter;
    auto key() const { return std::tie(job,item,role,filter); }
};

command_result ReadObservation(color_ostream &, const wire::Request *in, wire::Reply *out)
{
    if (!authorize(in,out)) return CR_OK;
    out->set_failure_code(4);
    auto *world=df::global::world;
    if (!Core::getInstance().isWorldLoaded() || !World::isFortressMode() || !world ||
        !df::global::job_next_id || !df::global::building_next_id || !df::global::item_next_id) return CR_OK;
    out->set_failure_code(5);
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick=World::ReadCurrentTick(); const auto site=World::GetCurrentSiteId();
    const auto next_job=*df::global::job_next_id, next_building=*df::global::building_next_id, next_item=*df::global::item_next_id;
    const std::string folder=World::ReadWorldFolder();
    if (year<0 || year>std::numeric_limits<std::uint32_t>::max() || tick>=403200 || site<0 ||
        next_job<0 || next_building<0 || next_item<0 || !valid_utf8(folder,512,false)) return CR_OK;
    // Both methods are registered with flags=0: all domains are read during the
    // same DFHack suspension. No unpause, nested RPC, or partial publication.
    if (world->buildings.all.size()>in->max_buildings() || world->items.all.size()>in->max_items()) {
        out->set_failure_code(3); return CR_OK;
    }
    std::vector<df::building *> buildings; std::map<std::int32_t,df::building *> building_ids;
    std::vector<df::item *> items; std::map<std::int32_t,df::item *> item_ids;
    if (!roster(world->buildings.all,in->max_buildings(),next_building,buildings,building_ids) ||
        !roster(world->items.all,in->max_items(),next_item,items,item_ids)) return CR_OK;
    std::vector<df::job *> jobs;
    for (auto *link=world->jobs.list.next; link; link=link->next) {
        if (jobs.size()>=in->max_jobs()) { out->set_failure_code(3); return CR_OK; }
        if (!link->item) return CR_OK;
        jobs.push_back(link->item);
    }
    std::sort(jobs.begin(),jobs.end(),[](const df::job *a,const df::job *b){return a->id<b->id;});
    std::string job_payload("DFMJ1200",8);
    u32(job_payload,static_cast<std::uint32_t>(year)); u32(job_payload,tick);
    job_payload.push_back(World::ReadPauseState() ? 1 : 0);
    i32(job_payload,site); u32(job_payload,static_cast<std::uint32_t>(next_job)); text(job_payload,folder);
    u32(job_payload,static_cast<std::uint32_t>(jobs.size()));
    std::vector<Attachment> attachments;
    std::int32_t previous=-1;
    for (auto *job:jobs) {
        if (job->id<=previous || job->id>=next_job || static_cast<int>(job->job_type)<0 || job->completion_timer< -1 ||
            job->job_items.elements.size()>4096 || job->general_refs.size()>4096) return CR_OK;
        if (job->items.size()>MAX_ATTACHMENTS-attachments.size()) { out->set_failure_code(3); return CR_OK; }
        for (const auto *ref:job->general_refs) if (!ref) return CR_OK;
        const std::string key=ENUM_KEY_STR(job_type,job->job_type);
        if (!valid_utf8(key,128,false) || !valid_utf8(job->reaction_name,128,true)) return CR_OK;
        auto *worker=Job::getWorker(job); auto *holder=Job::getHolder(job);
        if ((worker && worker->id<0) || !member(building_ids,holder)) return CR_OK;
        for (const auto *ref:job->items) {
            if (!ref || !ref->item || !member(item_ids,ref->item) || static_cast<int>(ref->role)<0 || ref->job_item_idx< -1 ||
                (ref->job_item_idx>=0 && static_cast<std::size_t>(ref->job_item_idx)>=job->job_items.elements.size())) return CR_OK;
            attachments.push_back({job->id,ref->item->id,static_cast<std::int32_t>(ref->role),ref->job_item_idx});
        }
        u32(job_payload,static_cast<std::uint32_t>(job->id)); i32(job_payload,static_cast<std::int32_t>(job->job_type));
        text(job_payload,key); text(job_payload,job->reaction_name);
        job_payload.push_back(job->flags.bits.suspend ? 1 : 0); job_payload.push_back(job->flags.bits.repeat ? 1 : 0);
        i32(job_payload,job->pos.x); i32(job_payload,job->pos.y); i32(job_payload,job->pos.z);
        reference(job_payload,worker ? worker->id : -1); reference(job_payload,holder ? holder->id : -1);
        i32(job_payload,job->completion_timer); u32(job_payload,static_cast<std::uint32_t>(job->items.size()));
        u32(job_payload,static_cast<std::uint32_t>(job->job_items.elements.size())); previous=job->id;
        if (job_payload.size()>in->max_bytes()) { out->set_failure_code(3); return CR_OK; }
    }
    std::string payload("DFMO1300",8); u32(payload,static_cast<std::uint32_t>(job_payload.size())); payload.append(job_payload);
    u32(payload,static_cast<std::uint32_t>(next_building)); u32(payload,static_cast<std::uint32_t>(next_item));
    u32(payload,static_cast<std::uint32_t>(buildings.size()));
    for (auto *b:buildings) {
        const auto type=b->getType(); const std::string key=ENUM_KEY_STR(building_type,type);
        const auto stage=b->getBuildStage(), maximum=b->getMaxBuildStage();
        if (static_cast<int>(type)<0 || !valid_utf8(key,128,false) || b->x1>b->x2 || b->y1>b->y2 || stage<0 || stage>maximum) return CR_OK;
        u32(payload,static_cast<std::uint32_t>(b->id)); i32(payload,static_cast<std::int32_t>(type)); text(payload,key);
        for (std::int32_t n:std::initializer_list<std::int32_t>{b->x1,b->y1,b->x2,b->y2,b->z}) i32(payload,n);
        i32(payload,stage); i32(payload,maximum);
        if (payload.size()>in->max_bytes()) { out->set_failure_code(3); return CR_OK; }
    }
    std::map<std::int32_t,std::int32_t> containers;
    u32(payload,static_cast<std::uint32_t>(items.size()));
    for (auto *item:items) {
        const auto type=item->getType(); const std::string key=ENUM_KEY_STR(item_type,type);
        const std::int32_t subtype=item->getSubtype(), material=item->getMaterial();
        const std::int32_t material_index=item->getMaterialIndex(), stack=item->getStackSize();
        if (static_cast<int>(type)<0 || !valid_utf8(key,128,false) || subtype< -1 || material< -1 || material_index< -1 || stack<0 ||
            item->general_refs.size()>4096) return CR_OK;
        for (const auto *ref:item->general_refs) if (!ref) return CR_OK;
        auto *container=Items::getContainer(item); auto *holder=Items::getHolderBuilding(item);
        if (!member(item_ids,container) || !member(building_ids,holder)) return CR_OK;
        if (container) containers.emplace(item->id,container->id);
        u32(payload,static_cast<std::uint32_t>(item->id)); i32(payload,static_cast<std::int32_t>(type)); text(payload,key);
        i32(payload,subtype); i32(payload,material); i32(payload,material_index); u32(payload,static_cast<std::uint32_t>(stack));
        // Deliberately raw item.pos: resolving nested containment is a separate query.
        i32(payload,item->pos.x); i32(payload,item->pos.y); i32(payload,item->pos.z);
        std::uint32_t flags=0; unsigned bit=0;
        for (bool value:{bool(item->flags.bits.forbid),bool(item->flags.bits.in_job),bool(item->flags.bits.dump),
            bool(item->flags.bits.removed),bool(item->flags.bits.rotten),bool(item->flags.bits.trader),
            bool(item->flags.bits.on_ground),bool(item->flags.bits.in_inventory),bool(item->flags.bits.in_building)}) {
            if (value) flags|=std::uint32_t{1}<<bit;
            ++bit;
        }
        u32(payload,flags); reference(payload,container ? container->id : -1); reference(payload,holder ? holder->id : -1);
        if (payload.size()>in->max_bytes()) { out->set_failure_code(3); return CR_OK; }
    }
    std::map<std::int32_t,unsigned char> colors;
    for (auto *item:items) {
        auto id=item->id; std::vector<std::int32_t> chain;
        while (id>=0) {
            if (colors[id]==1) return CR_OK;
            if (colors[id]==2) break;
            colors[id]=1; chain.push_back(id);
            const auto found=containers.find(id); id=found==containers.end() ? -1 : found->second;
        }
        for (auto value:chain) colors[value]=2;
    }
    std::sort(attachments.begin(),attachments.end(),[](const Attachment &a,const Attachment &b){return a.key()<b.key();});
    u32(payload,static_cast<std::uint32_t>(attachments.size()));
    for (std::size_t n=0;n<attachments.size();++n) {
        const auto &a=attachments[n]; if (n && attachments[n-1].key()==a.key()) return CR_OK;
        i32(payload,a.job); i32(payload,a.item); i32(payload,a.role); i32(payload,a.filter);
        if (payload.size()>in->max_bytes()) { out->set_failure_code(3); return CR_OK; }
    }
    if (payload.size()>in->max_bytes()) { out->set_failure_code(3); return CR_OK; }
    out->set_observation(payload); out->set_failure_code(0); out->set_accepted(true); return CR_OK;
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event)
{
    if ((event==SC_WORLD_LOADED || event==SC_WORLD_UNLOADED) && generation!=std::numeric_limits<std::uint64_t>::max()) ++generation;
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &)
{
    auto *service=new RPCService(); service->addFunction("Handshake",Handshake,0);
    service->addFunction("ReadObservation",ReadObservation,0); return service;
}
