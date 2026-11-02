#pragma once
// Explicit component doubles. No generated protobuf, DFHack ABI or game behavior claim.
#include "../workshop/stubs.h"
namespace df {
struct plant_tree_tile {std::uint16_t whole=0;};
struct plant_tree_info {plant_tree_tile **body=nullptr;std::int32_t dim_x=0,dim_y=0,body_height=0;};
struct plant {
    coord pos;std::int16_t material=3;plant_tree_info *tree_info=nullptr;
    union Damage {std::uint32_t whole;struct {std::uint32_t unused:1,season_dead:1,dead:1,other:29;}bits;Damage():whole(0){}} damage_flags;
};
}
namespace mock {
enum class MarkFault {None,Before,DigOnly,BlockOnly,FalseAfter,ThrowAfter};
inline MarkFault mark_fault=MarkFault::None;
inline std::function<void()> on_plant_lookup;
}
namespace DFHack::Designations {
inline df::coord getPlantDesignationTile(const df::plant *plant) {
    ++mock::helper_calls;if(mock::on_plant_lookup)mock::on_plant_lookup();
    if(!plant->tree_info)return plant->pos;
    const auto *t=plant->tree_info;const int cx=t->dim_x/2,cy=t->dim_y/2;int x=cx,y=cy;
    while(x+1<t->dim_x&&y+1<t->dim_y) {
        if(t->body[0][y*t->dim_x+x+1].whole&1)++x;
        else if(t->body[0][(y+1)*t->dim_x+x].whole&1)++y;
        else break;
    }
    return {plant->pos.x-cx+x,plant->pos.y-cy+y,plant->pos.z};
}
inline bool isPlantMarked(const df::plant *plant) {
    const auto p=getPlantDesignationTile(plant);const auto *b=Maps::getTileBlock(p);
    if(!b)return false;
    if(b->designation[p.x&15][p.y&15].bits.dig==df::tile_dig_designation::Default)return true;
    for(auto *link=df::global::world->jobs.list.next;link;link=link->next)
        if(link->item&&(link->item->job_type==df::job_type::FellTree||link->item->job_type==df::job_type::GatherPlants)&&link->item->pos==p)return true;
    return false;
}
inline bool markPlant(const df::plant *plant) {
    ++mock::writer_calls;
    if(mock::mark_fault==mock::MarkFault::Before)return false;
    if(isPlantMarked(plant))return false;
    const auto p=getPlantDesignationTile(plant);auto *b=Maps::getTileBlock(p);if(!b)return false;
    if(mock::mark_fault!=mock::MarkFault::BlockOnly)b->designation[p.x&15][p.y&15].bits.dig=df::tile_dig_designation::Default;
    if(mock::mark_fault!=mock::MarkFault::DigOnly)b->flags.bits.designated=true;
    if(mock::mark_fault==mock::MarkFault::ThrowAfter)throw std::runtime_error("after designation");
    return mock::mark_fault!=mock::MarkFault::FalseAfter;
}
}
namespace dfmcp::tree_harvest::v1_21 {
struct Request {
    unsigned mask=0;std::string bearer=std::string(32,'s'),nonce=std::string(16,'n');unsigned major=1,minor=21;
    std::uint32_t xx=15,yy=15,zz=2,since=1000;std::int32_t tree_species=3;
    std::string key="tree-001",witness,plan,token;
    struct Fields {bool empty()const{return !mock::unknown;}};
    struct Reflection {Fields GetUnknownFields(const Request&)const{return {};}};
    const Reflection *GetReflection()const{static Reflection value;return &value;}
    bool IsInitialized()const{return mock::initialized;}
    std::size_t ByteSize()const{return mock::oversized_request?2049:128+bearer.size()+nonce.size()+key.size()+witness.size()+plan.size()+token.size();}
    const std::string &bearer_token()const{return bearer;}const std::string &client_nonce()const{return nonce;}
    unsigned protocol_major()const{return major;}unsigned protocol_minor()const{return minor;}
#define TH_FIELD(type,name,member,bit) bool has_##name()const{return(mask&(bit))!=0;}type name()const{return member;}
    TH_FIELD(std::uint32_t,x,xx,1)
    TH_FIELD(std::uint32_t,y,yy,2)
    TH_FIELD(std::uint32_t,z,zz,4)
    TH_FIELD(const std::string&,idempotency_key,key,8)
    TH_FIELD(const std::string&,expected_witness,witness,16)
    TH_FIELD(const std::string&,plan_digest,plan,32)
    TH_FIELD(const std::string&,prepare_token,token,64)
    TH_FIELD(std::uint32_t,since_item_id,since,128)
    TH_FIELD(std::int32_t,species,tree_species,256)
#undef TH_FIELD
};
struct Reply:dfmcp::workshop::v1_20::Reply {
    std::size_t ByteSize()const{return mock::oversized_reply?512*1024+1:128+nonce.size()+df_version.size()+dfhack_version.size()+observation.size()+effect.size();}
};
}
