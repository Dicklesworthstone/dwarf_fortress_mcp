#pragma once
#include <algorithm>
#include <cstdint>
#include <limits>
#include <map>
#include <string>
#include <tuple>
#include <vector>
#include "Core.h"
#include "MiscUtils.h"
#include "TileTypes.h"
#include "modules/Job.h"
#include "modules/Items.h"
#include "modules/Maps.h"
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
#include "df/map_block.h"
#include "df/tile_designation.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_shape.h"

// Data-only component codecs used within a SINGLE native RPC suspension.
// They perform no RPC, cache publication, game mutation, or background work.
namespace dfmcp_spatial_capture {
using namespace DFHack;
inline void u32(std::string &out,std::uint32_t v){for(int s=24;s>=0;s-=8)out.push_back(static_cast<char>((v>>s)&255));}
inline void i32(std::string &out,std::int32_t v){u32(out,static_cast<std::uint32_t>(v));}
inline void u16(std::string &out,std::uint16_t v){out.push_back(static_cast<char>(v>>8));out.push_back(static_cast<char>(v&255));}
inline void text(std::string &out,const std::string &v){u16(out,static_cast<std::uint16_t>(v.size()));out+=v;}
inline void reference(std::string &out,std::int32_t v){out.push_back(v>=0?1:0);if(v>=0)u32(out,static_cast<std::uint32_t>(v));}
inline bool utf8(const std::string &v,std::size_t maximum,bool empty=false){
    if(v.size()>maximum||(!empty&&v.empty()))return false;
    for(std::size_t i=0;i<v.size();){
        const auto c=static_cast<unsigned char>(v[i]);if(!c)return false;if(c<128){++i;continue;}
        const std::size_t n=c>=0xc2&&c<=0xdf?2:c>=0xe0&&c<=0xef?3:c>=0xf0&&c<=0xf4?4:0;
        if(!n||i+n>v.size())return false;
        for(std::size_t j=1;j<n;++j)if((static_cast<unsigned char>(v[i+j])&0xc0)!=0x80)return false;
        const auto b=static_cast<unsigned char>(v[i+1]);
        if((c==0xe0&&b<0xa0)||(c==0xed&&b>=0xa0)||(c==0xf0&&b<0x90)||(c==0xf4&&b>=0x90))return false;
        i+=n;
    }return true;
}
struct Bounds {
    std::uint32_t jobs,buildings,items,bytes;
    std::uint32_t origin[3],size[3];
    bool valid() const {
        if(!jobs||jobs>4096||!buildings||buildings>4096||!items||items>65536||bytes<1024||bytes>16*1024*1024)return false;
        std::uint64_t volume=1;
        for(unsigned a=0;a<3;++a){if(!size[a]||size[a]>128||origin[a]>=32768||size[a]>32768-origin[a])return false;volume*=size[a];}
        return volume<=16384;
    }
};
inline std::uint32_t world_header(const char *magic,std::string &out,bool horizon){
    if(!Core::getInstance().isWorldLoaded()||!World::isFortressMode())return 4;
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick=World::ReadCurrentTick();const auto site=World::GetCurrentSiteId();const auto folder=World::ReadWorldFolder();
    if(year<0||year>std::numeric_limits<std::uint32_t>::max()||tick>=403200||site<0||!utf8(folder,512))return 5;
    out.assign(magic,8);u32(out,static_cast<std::uint32_t>(year));u32(out,tick);out.push_back(World::ReadPauseState()?1:0);
    i32(out,site);if(horizon){if(!df::global::job_next_id||*df::global::job_next_id<0)return 5;u32(out,*df::global::job_next_id);}
    text(out,folder);return 0;
}
template<class T>bool roster(const std::vector<T *> &input,std::uint32_t maximum,std::int32_t horizon,std::map<std::int32_t,T *> &out){
    if(input.size()>maximum||horizon<0)return false;
    for(auto *v:input)if(!v||v->id<0||v->id>=horizon||!out.emplace(v->id,v).second)return false;
    return true;
}
template<class T>bool member(const std::map<std::int32_t,T *> &values,T *v){
    if(!v)return true;
    const auto found=values.find(v->id);return found!=values.end()&&found->second==v;
}
struct Attachment{std::int32_t job,item,role,filter;auto key()const{return std::tie(job,item,role,filter);}};
inline std::uint32_t operations(const Bounds &b,std::string &payload){
    auto *world=df::global::world;
    if(!world||!df::global::building_next_id||!df::global::item_next_id)return 4;
    std::string jp;const auto status=world_header("DFMJ1200",jp,true);if(status)return status;
    const auto nj=*df::global::job_next_id,nb=*df::global::building_next_id,ni=*df::global::item_next_id;
    if(world->buildings.all.size()>b.buildings||world->items.all.size()>b.items)return 3;
    std::map<std::int32_t,df::building *> buildings;std::map<std::int32_t,df::item *> items;
    if(!roster(world->buildings.all,b.buildings,nb,buildings)||!roster(world->items.all,b.items,ni,items))return 5;
    std::map<std::int32_t,df::job *> jobs;
    for(auto *link=world->jobs.list.next;link;link=link->next){
        if(jobs.size()>=b.jobs)return 3;
        auto *j=link->item;if(!j||j->id<0||j->id>=nj||!jobs.emplace(j->id,j).second)return 5;
    }
    u32(jp,static_cast<std::uint32_t>(jobs.size()));std::vector<Attachment> attachments;
    for(const auto &entry:jobs){auto *j=entry.second;
        if(static_cast<int>(j->job_type)<0||j->completion_timer< -1||j->job_items.elements.size()>4096||j->general_refs.size()>4096)return 5;
        if(j->items.size()>65536-attachments.size())return 3;
        for(const auto *ref:j->general_refs)if(!ref)return 5;
        const std::string key=ENUM_KEY_STR(job_type,j->job_type);if(!utf8(key,128)||!utf8(j->reaction_name,128,true))return 5;
        auto *worker=Job::getWorker(j);auto *holder=Job::getHolder(j);if((worker&&worker->id<0)||!member(buildings,holder))return 5;
        for(const auto *ref:j->items){
            if(!ref||!ref->item||!member(items,ref->item)||static_cast<int>(ref->role)<0||ref->job_item_idx< -1||
                (ref->job_item_idx>=0&&static_cast<std::size_t>(ref->job_item_idx)>=j->job_items.elements.size()))return 5;
            attachments.push_back({j->id,ref->item->id,static_cast<std::int32_t>(ref->role),ref->job_item_idx});
        }
        u32(jp,j->id);i32(jp,static_cast<std::int32_t>(j->job_type));text(jp,key);text(jp,j->reaction_name);
        jp.push_back(j->flags.bits.suspend?1:0);jp.push_back(j->flags.bits.repeat?1:0);
        i32(jp,j->pos.x);i32(jp,j->pos.y);i32(jp,j->pos.z);reference(jp,worker?worker->id:-1);reference(jp,holder?holder->id:-1);
        i32(jp,j->completion_timer);u32(jp,static_cast<std::uint32_t>(j->items.size()));u32(jp,static_cast<std::uint32_t>(j->job_items.elements.size()));
        if(jp.size()>2*1024*1024||jp.size()>b.bytes)return 3;
    }
    payload.assign("DFMO1400",8);u32(payload,static_cast<std::uint32_t>(jp.size()));payload+=jp;u32(payload,nb);u32(payload,ni);
    u32(payload,static_cast<std::uint32_t>(buildings.size()));
    for(const auto &entry:buildings){auto *v=entry.second;
        const auto type=v->getType();const std::string key=ENUM_KEY_STR(building_type,type);const auto stage=v->getBuildStage(),maximum=v->getMaxBuildStage();
        if(static_cast<int>(type)<0||!utf8(key,128)||v->x1>v->x2||v->y1>v->y2||stage<0||stage>maximum)return 5;
        u32(payload,v->id);i32(payload,static_cast<std::int32_t>(type));text(payload,key);
        for(std::int32_t n:std::initializer_list<std::int32_t>{v->x1,v->y1,v->x2,v->y2,v->z,stage,maximum})i32(payload,n);
        if(payload.size()>b.bytes)return 3;
    }
    u32(payload,static_cast<std::uint32_t>(items.size()));std::map<std::int32_t,std::int32_t> containers;
    for(const auto &entry:items){auto *v=entry.second;
        const auto type=v->getType();const std::string key=ENUM_KEY_STR(item_type,type);
        const std::int32_t subtype=v->getSubtype(),mat=v->getMaterial(),index=v->getMaterialIndex(),stack=v->getStackSize();
        if(static_cast<int>(type)<0||!utf8(key,128)||subtype< -1||mat< -1||index< -1||stack<0||v->general_refs.size()>4096)return 5;
        for(const auto *ref:v->general_refs)if(!ref)return 5;
        auto *container=Items::getContainer(v);auto *holder=Items::getHolderBuilding(v);
        if(!member(items,container)||!member(buildings,holder))return 5;
        if(container)containers.emplace(v->id,container->id);
        u32(payload,v->id);i32(payload,static_cast<std::int32_t>(type));text(payload,key);i32(payload,subtype);i32(payload,mat);i32(payload,index);u32(payload,stack);
        i32(payload,v->pos.x);i32(payload,v->pos.y);i32(payload,v->pos.z);std::uint32_t flags=0;unsigned bit=0;
        for(bool flag:{bool(v->flags.bits.forbid),bool(v->flags.bits.in_job),bool(v->flags.bits.dump),bool(v->flags.bits.removed),
            bool(v->flags.bits.rotten),bool(v->flags.bits.trader),bool(v->flags.bits.on_ground),bool(v->flags.bits.in_inventory),bool(v->flags.bits.in_building)}){
            if(flag)flags|=std::uint32_t{1}<<bit;
            ++bit;
        }
        u32(payload,flags);reference(payload,container?container->id:-1);reference(payload,holder?holder->id:-1);
        if(payload.size()>b.bytes)return 3;
    }
    std::map<std::int32_t,unsigned char> colors;
    for(const auto &entry:items){auto id=entry.first;std::vector<std::int32_t> chain;
        while(id>=0){if(colors[id]==1)return 5;if(colors[id]==2)break;colors[id]=1;chain.push_back(id);
            const auto parent=containers.find(id);id=parent==containers.end()?-1:parent->second;}
        for(auto id_in_chain:chain)colors[id_in_chain]=2;
    }
    std::sort(attachments.begin(),attachments.end(),[](const Attachment &a,const Attachment &v){return a.key()<v.key();});
    u32(payload,static_cast<std::uint32_t>(attachments.size()));
    for(std::size_t i=0;i<attachments.size();++i){const auto &a=attachments[i];if(i&&attachments[i-1].key()==a.key())return 5;
        i32(payload,a.job);i32(payload,a.item);i32(payload,a.role);i32(payload,a.filter);if(payload.size()>b.bytes)return 3;}
    return payload.size()>b.bytes?3:0;
}
inline std::uint8_t shape_tag(df::tiletype_shape shape){switch(shape){
    case df::tiletype_shape::EMPTY:return 1;case df::tiletype_shape::WALL:return 2;case df::tiletype_shape::FLOOR:return 3;
    case df::tiletype_shape::RAMP:return 4;case df::tiletype_shape::RAMP_TOP:return 5;case df::tiletype_shape::STAIR_UP:return 6;
    case df::tiletype_shape::STAIR_DOWN:return 7;case df::tiletype_shape::STAIR_UPDOWN:return 8;default:return 0;}}
inline std::uint32_t terrain(const Bounds &b,std::string &out){
    const auto status=world_header("DFMM1500",out,false);if(status)return status;
    std::int32_t dimensions[3]={};Maps::getTileSize(dimensions[0],dimensions[1],dimensions[2]);
    for(unsigned a=0;a<3;++a)if(dimensions[a]<=0||dimensions[a]>32768||b.origin[a]+b.size[a]>static_cast<std::uint32_t>(dimensions[a]))return 5;
    for(auto n:dimensions)u32(out,n);
    for(auto n:b.origin)u32(out,n);
    for(auto n:b.size)u32(out,n);
    u32(out,b.size[0]*b.size[1]*b.size[2]);
    for(std::uint32_t z=b.origin[2];z<b.origin[2]+b.size[2];++z)
    for(std::uint32_t y=b.origin[1];y<b.origin[1]+b.size[1];++y)
    for(std::uint32_t x=b.origin[0];x<b.origin[0]+b.size[0];++x){
        auto *block=Maps::getTileBlock(x,y,z);if(!block){out.push_back(0);continue;}
        const auto lx=x&15,ly=y&15;const auto &d=block->designation[lx][ly].bits;
        if(d.hidden){out.push_back(1);continue;}
        const auto tt=block->tiletype[lx][ly];if(static_cast<int>(tt)<0||!is_valid_enum_item(tt))return 5;
        const auto &o=block->occupancy[lx][ly].bits;out.push_back(2);u32(out,static_cast<std::uint32_t>(tt));out.push_back(static_cast<char>(shape_tag(tileShape(tt))));
        out.push_back(static_cast<char>(d.flow_size));out.push_back(d.liquid_type?1:0);out.push_back(static_cast<char>(d.traffic));out.push_back(static_cast<char>(d.dig));
        out.push_back(static_cast<char>(o.building));out.push_back(static_cast<char>((o.unit?1:0)|(o.unit_grounded?2:0)));
        u32(out,block->walkable[lx][ly]);u16(out,block->temperature_1[lx][ly]);u16(out,block->temperature_2[lx][ly]);
    }
    return out.size()>1024*1024||out.size()>b.bytes?3:0;
}
inline std::uint32_t capture(const Bounds &b,std::string &out){
    if(!b.valid())return 3;
    std::string op,map;auto status=operations(b,op);if(status)return status;status=terrain(b,map);if(status)return status;
    if(op.size()+map.size()+16>b.bytes)return 3;
    out.assign("DFMS1600",8);u32(out,static_cast<std::uint32_t>(op.size()));out+=op;u32(out,static_cast<std::uint32_t>(map.size()));out+=map;return 0;
}
}
