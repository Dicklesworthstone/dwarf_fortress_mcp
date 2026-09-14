#include "../common/retained_snapshot.h"
#include <cstdlib>
#include <tuple>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "MiscUtils.h"
#include "DfmcpOperationsV1_4.pb.h"
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
namespace wire=dfmcp::operations::v1_4;
DFHACK_PLUGIN("dfmcp_operations_v1_4");
namespace {
constexpr std::size_t MAX_JOBS=4096,MAX_BUILDINGS=4096,MAX_ITEMS=65536,MAX_ATTACHMENTS=65536;
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|std::uint64_t{1};
dfmcp_snapshot::Cache snapshots;

bool valid_utf8(const std::string &value,std::size_t maximum,bool empty)
{
    if(value.size()>maximum || (!empty && value.empty())) return false;
    for(std::size_t i=0;i<value.size();) {
        const auto c=static_cast<unsigned char>(value[i]);
        if(!c) return false;
        if(c<128) {++i;continue;}
        const std::size_t n=c>=0xc2 && c<=0xdf ? 2 : c>=0xe0 && c<=0xef ? 3 : c>=0xf0 && c<=0xf4 ? 4 : 0;
        if(!n || i+n>value.size()) return false;
        for(std::size_t j=1;j<n;++j) if((static_cast<unsigned char>(value[i+j])&0xc0)!=0x80) return false;
        const auto second=static_cast<unsigned char>(value[i+1]);
        if((c==0xe0 && second<0xa0)||(c==0xed && second>=0xa0)||(c==0xf0 && second<0x90)||(c==0xf4 && second>=0x90)) return false;
        i+=n;
    }
    return true;
}
void u32(std::string &out,std::uint32_t v) {for(int shift=24;shift>=0;shift-=8)out.push_back(static_cast<char>((v>>shift)&255));}
void i32(std::string &out,std::int32_t v) {u32(out,static_cast<std::uint32_t>(v));}
void text(std::string &out,const std::string &v) {
    out.push_back(static_cast<char>((v.size()>>8)&255));out.push_back(static_cast<char>(v.size()&255));out.append(v);
}
void reference(std::string &out,std::int32_t v) {out.push_back(v>=0 ? 1 : 0);if(v>=0)u32(out,static_cast<std::uint32_t>(v));}
dfmcp_snapshot::Limits bounds(const wire::Request *in) {return {in->max_jobs(),in->max_buildings(),in->max_items(),in->max_bytes()};}

bool authorize(const wire::Request *in,wire::Reply *out)
{
    out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce("");
    out->set_protocol_major(1);out->set_protocol_minor(4);out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
    if(in->client_nonce().size()<16 || in->client_nonce().size()>64 || !in->max_jobs() || in->max_jobs()>MAX_JOBS ||
        !in->max_buildings() || in->max_buildings()>MAX_BUILDINGS || !in->max_items() || in->max_items()>MAX_ITEMS ||
        in->max_bytes()<1024 || in->max_bytes()>dfmcp_snapshot::Cache::MAX_CAPTURE_BYTES ||
        in->page_bytes()<dfmcp_snapshot::Cache::MIN_PAGE || in->page_bytes()>dfmcp_snapshot::Cache::MAX_PAGE ||
        (!in->snapshot_token().empty() && in->snapshot_token().size()!=16)) return false;
    out->set_client_nonce(in->client_nonce());
    if(in->protocol_major()!=1 || in->protocol_minor()!=4) {out->set_failure_code(2);return false;}
    const char *configured=std::getenv("DFMCP_OPERATIONS_PAGED_TOKEN");
    const std::string_view expected=configured ? std::string_view(configured) : std::string_view();
    const auto &presented=in->bearer_token();out->set_failure_code(1);
    if(expected.size()<32 || expected.size()>256 || presented.size()<32 || presented.size()>256) return false;
    std::size_t difference=expected.size()^presented.size();
    for(std::size_t i=0;i<256;++i) {
        const unsigned char a=i<expected.size()?expected[i]:0,b=i<presented.size()?presented[i]:0;
        difference|=static_cast<std::size_t>(a^b);
    }
    if(difference) return false;
    out->set_failure_code(5);
    const auto &version=Core::getInstance().vinfo;
    const std::string df=version?version->getVersion():std::string(),dfhack=Version::dfhack_version();
    if(!generation || generation==std::numeric_limits<std::uint64_t>::max() || !valid_utf8(df,128,false) || !valid_utf8(dfhack,128,false)) return false;
    out->set_bridge_generation(generation);out->set_df_version(df);out->set_dfhack_version(dfhack);out->set_failure_code(0);return true;
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out)
{
    if(authorize(in,out)) {
        if(!in->snapshot_token().empty() || in->offset()!=0 || in->release())out->set_failure_code(3);
        else out->set_accepted(true);
    }
    return CR_OK;
}

template<class T> bool roster(const std::vector<T *> &source,std::size_t maximum,std::int32_t horizon,
    std::vector<T *> &ordered,std::map<std::int32_t,T *> &by_id)
{
    if(source.size()>maximum || horizon<0)return false;
    for(auto *v:source)if(!v || v->id<0 || v->id>=horizon || !by_id.emplace(v->id,v).second)return false;
    for(const auto &v:by_id)ordered.push_back(v.second);
    return true;
}
template<class T>bool member(const std::map<std::int32_t,T *> &values,T *v)
{
    if(!v)return true;
    const auto found=values.find(v->id);return found!=values.end() && found->second==v;
}
struct Attachment {std::int32_t job,item,role,filter;auto key()const{return std::tie(job,item,role,filter);}};

// Returns only bytes. All game access happens in this call under RPC flags=0.
std::uint32_t capture(const wire::Request *in,std::string &payload)
{
    auto *world=df::global::world;
    if(!Core::getInstance().isWorldLoaded() || !World::isFortressMode() || !world || !df::global::job_next_id ||
        !df::global::building_next_id || !df::global::item_next_id)return 4;
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());const auto tick=World::ReadCurrentTick();const auto site=World::GetCurrentSiteId();
    const auto nj=*df::global::job_next_id,nb=*df::global::building_next_id,ni=*df::global::item_next_id;
    const std::string folder=World::ReadWorldFolder();
    if(year<0 || year>std::numeric_limits<std::uint32_t>::max() || tick>=403200 || site<0 || nj<0 || nb<0 || ni<0 || !valid_utf8(folder,512,false))return 5;
    if(world->items.all.size()>in->max_items() || world->buildings.all.size()>in->max_buildings())return 3;
    std::vector<df::building *> buildings;std::map<std::int32_t,df::building *> building_ids;
    std::vector<df::item *> items;std::map<std::int32_t,df::item *> item_ids;
    if(!roster(world->buildings.all,in->max_buildings(),nb,buildings,building_ids) || !roster(world->items.all,in->max_items(),ni,items,item_ids))return 5;
    std::vector<df::job *> jobs;
    for(auto *link=world->jobs.list.next;link;link=link->next) {
        if(jobs.size()>=in->max_jobs())return 3;
        if(!link->item)return 5;
        jobs.push_back(link->item);
    }
    std::sort(jobs.begin(),jobs.end(),[](const df::job *a,const df::job *b){return a->id<b->id;});
    std::string jp("DFMJ1200",8);u32(jp,static_cast<std::uint32_t>(year));u32(jp,tick);jp.push_back(World::ReadPauseState()?1:0);
    i32(jp,site);u32(jp,static_cast<std::uint32_t>(nj));text(jp,folder);u32(jp,static_cast<std::uint32_t>(jobs.size()));
    std::vector<Attachment> attachments;std::int32_t previous=-1;
    for(auto *job:jobs) {
        if(job->id<=previous || job->id>=nj || static_cast<int>(job->job_type)<0 || job->completion_timer< -1 ||
            job->job_items.elements.size()>4096 || job->general_refs.size()>4096)return 5;
        if(job->items.size()>MAX_ATTACHMENTS-attachments.size())return 3;
        for(const auto *ref:job->general_refs)if(!ref)return 5;
        const std::string key=ENUM_KEY_STR(job_type,job->job_type);
        if(!valid_utf8(key,128,false) || !valid_utf8(job->reaction_name,128,true))return 5;
        auto *worker=Job::getWorker(job);auto *holder=Job::getHolder(job);
        if((worker && worker->id<0) || !member(building_ids,holder))return 5;
        for(const auto *ref:job->items) {
            if(!ref || !ref->item || !member(item_ids,ref->item) || static_cast<int>(ref->role)<0 || ref->job_item_idx< -1 ||
                (ref->job_item_idx>=0 && static_cast<std::size_t>(ref->job_item_idx)>=job->job_items.elements.size()))return 5;
            attachments.push_back({job->id,ref->item->id,static_cast<std::int32_t>(ref->role),ref->job_item_idx});
        }
        u32(jp,static_cast<std::uint32_t>(job->id));i32(jp,static_cast<std::int32_t>(job->job_type));text(jp,key);text(jp,job->reaction_name);
        jp.push_back(job->flags.bits.suspend?1:0);jp.push_back(job->flags.bits.repeat?1:0);
        i32(jp,job->pos.x);i32(jp,job->pos.y);i32(jp,job->pos.z);reference(jp,worker?worker->id:-1);reference(jp,holder?holder->id:-1);
        i32(jp,job->completion_timer);u32(jp,static_cast<std::uint32_t>(job->items.size()));u32(jp,static_cast<std::uint32_t>(job->job_items.elements.size()));
        previous=job->id;if(jp.size()>2*1024*1024 || jp.size()>in->max_bytes())return 3;
    }
    payload.assign("DFMO1400",8);u32(payload,static_cast<std::uint32_t>(jp.size()));payload.append(jp);
    u32(payload,static_cast<std::uint32_t>(nb));u32(payload,static_cast<std::uint32_t>(ni));u32(payload,static_cast<std::uint32_t>(buildings.size()));
    for(auto *b:buildings) {
        const auto type=b->getType();const std::string key=ENUM_KEY_STR(building_type,type);const auto stage=b->getBuildStage(),maximum=b->getMaxBuildStage();
        if(static_cast<int>(type)<0 || !valid_utf8(key,128,false) || b->x1>b->x2 || b->y1>b->y2 || stage<0 || stage>maximum)return 5;
        u32(payload,static_cast<std::uint32_t>(b->id));i32(payload,static_cast<std::int32_t>(type));text(payload,key);
        for(std::int32_t n:std::initializer_list<std::int32_t>{b->x1,b->y1,b->x2,b->y2,b->z})i32(payload,n);
        i32(payload,stage);i32(payload,maximum);if(payload.size()>in->max_bytes())return 3;
    }
    std::map<std::int32_t,std::int32_t> containers;u32(payload,static_cast<std::uint32_t>(items.size()));
    for(auto *item:items) {
        const auto type=item->getType();const std::string key=ENUM_KEY_STR(item_type,type);
        const std::int32_t subtype=item->getSubtype(),mat=item->getMaterial(),index=item->getMaterialIndex(),stack=item->getStackSize();
        if(static_cast<int>(type)<0 || !valid_utf8(key,128,false) || subtype< -1 || mat< -1 || index< -1 || stack<0 || item->general_refs.size()>4096)return 5;
        for(const auto *ref:item->general_refs)if(!ref)return 5;
        auto *container=Items::getContainer(item);auto *holder=Items::getHolderBuilding(item);
        if(!member(item_ids,container) || !member(building_ids,holder))return 5;
        if(container)containers.emplace(item->id,container->id);
        u32(payload,static_cast<std::uint32_t>(item->id));i32(payload,static_cast<std::int32_t>(type));text(payload,key);
        i32(payload,subtype);i32(payload,mat);i32(payload,index);u32(payload,static_cast<std::uint32_t>(stack));
        i32(payload,item->pos.x);i32(payload,item->pos.y);i32(payload,item->pos.z);
        std::uint32_t flags=0;unsigned bit=0;
        for(bool value:{bool(item->flags.bits.forbid),bool(item->flags.bits.in_job),bool(item->flags.bits.dump),bool(item->flags.bits.removed),
            bool(item->flags.bits.rotten),bool(item->flags.bits.trader),bool(item->flags.bits.on_ground),bool(item->flags.bits.in_inventory),bool(item->flags.bits.in_building)}) {
            if(value)flags|=std::uint32_t{1}<<bit;
            ++bit;
        }
        u32(payload,flags);reference(payload,container?container->id:-1);reference(payload,holder?holder->id:-1);
        if(payload.size()>in->max_bytes())return 3;
    }
    std::map<std::int32_t,unsigned char> colors;
    for(auto *item:items) {
        auto id=item->id;std::vector<std::int32_t> chain;
        while(id>=0) {
            if(colors[id]==1)return 5;
            if(colors[id]==2)break;
            colors[id]=1;chain.push_back(id);const auto found=containers.find(id);id=found==containers.end()?-1:found->second;
        }
        for(auto v:chain)colors[v]=2;
    }
    std::sort(attachments.begin(),attachments.end(),[](const Attachment &a,const Attachment &b){return a.key()<b.key();});
    u32(payload,static_cast<std::uint32_t>(attachments.size()));
    for(std::size_t n=0;n<attachments.size();++n) {
        const auto &a=attachments[n];if(n && attachments[n-1].key()==a.key())return 5;
        i32(payload,a.job);i32(payload,a.item);i32(payload,a.role);i32(payload,a.filter);
        if(payload.size()>in->max_bytes())return 3;
    }
    return payload.size()>in->max_bytes()?3:0;
}
command_result ReadObservation(color_ostream &,const wire::Request *in,wire::Reply *out)
{
    if(!authorize(in,out))return CR_OK;
    const auto now=dfmcp_snapshot::Cache::Clock::now();
    std::string token=in->snapshot_token();
    if(in->release()) {
        if(token.empty() || in->offset()!=0) {out->set_failure_code(3);return CR_OK;}
        if(!snapshots.release(in->client_nonce(),token,generation,bounds(in),now)) {out->set_failure_code(6);return CR_OK;}
        out->set_snapshot_token(token);out->set_accepted(true);return CR_OK;
    }
    if(token.empty()) {
        if(in->offset()!=0) {out->set_failure_code(3);return CR_OK;}
        if(!snapshots.can_capture(in->max_bytes(),now)) {out->set_failure_code(3);return CR_OK;}
        std::string payload;
        const auto code=capture(in,payload);if(code) {out->set_failure_code(code);return CR_OK;}
        if(!snapshots.insert(in->client_nonce(),generation,bounds(in),std::move(payload),
            dfmcp_snapshot::Cache::Clock::now(),token)) {out->set_failure_code(3);return CR_OK;}
    }
    dfmcp_snapshot::Page page;
    if(!snapshots.page(in->client_nonce(),token,generation,bounds(in),in->offset(),in->page_bytes(),
        dfmcp_snapshot::Cache::Clock::now(),page)) {out->set_failure_code(6);return CR_OK;}
    out->set_snapshot_token(page.token);out->set_offset(page.offset);out->set_total_bytes(page.total);
    out->set_payload_sha256(page.digest);out->set_complete(page.complete);out->set_observation(page.bytes);out->set_accepted(true);
    return CR_OK;
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &) {return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &) {snapshots.clear();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event)
{
    if(event==SC_WORLD_LOADED || event==SC_WORLD_UNLOADED) {
        snapshots.clear();if(generation!=std::numeric_limits<std::uint64_t>::max())++generation;
    }
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &)
{
    auto *service=new RPCService();service->addFunction("Handshake",Handshake,0);service->addFunction("ReadObservation",ReadObservation,0);return service;
}
