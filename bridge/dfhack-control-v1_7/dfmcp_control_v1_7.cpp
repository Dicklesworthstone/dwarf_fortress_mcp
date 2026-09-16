#include "../common/retained_snapshot.h"
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <map>
#include <string>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/World.h"
#include "DfmcpControlV1_7.pb.h"
using namespace DFHack;
namespace wire=dfmcp::control::v1_7;
DFHACK_PLUGIN("dfmcp_control_v1_7");
namespace {
constexpr std::size_t MAX_RECORDS=4096;
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|1;
struct Record {
    // Preparation identity must survive a later observation at another game tick.
    std::string digest,token,receipt;
    bool desired_paused=false;
    std::uint64_t expected_tick=0;
    bool requested=false,applied=false,paused=false;
    std::uint64_t tick=0;
};
std::map<std::string,Record> records;
bool text_ok(const std::string &v,std::size_t max){return !v.empty()&&v.size()<=max&&v.find('\0')==std::string::npos;}
void append_u64(std::string &out,std::uint64_t v){for(int s=56;s>=0;s-=8)out.push_back(static_cast<char>((v>>s)&255));}
std::string hash_token(const std::string &key,const std::string &digest,std::uint64_t tick,bool paused){
    std::string input="dfmcp-control-token-v2";input.push_back('\0');append_u64(input,generation);input+=key;input.push_back('\0');input+=digest;append_u64(input,tick);input.push_back(paused?1:0);
    const auto full=dfmcp_snapshot::sha256(input);return full.substr(0,16);
}
std::string receipt(const std::string &key,const std::string &digest,bool paused,std::uint64_t tick){
    std::string input="dfmcp-control-receipt-v2";input.push_back('\0');append_u64(input,generation);input+=key;input.push_back('\0');input+=digest;input.push_back(paused?1:0);append_u64(input,tick);
    return dfmcp_snapshot::sha256(input);
}
bool clock_tick(std::uint64_t &now) {
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick=World::ReadCurrentTick();
    if(year<0 || year>static_cast<std::int64_t>(std::numeric_limits<std::uint32_t>::max()) || tick>=403200) return false;
    now=static_cast<std::uint64_t>(year)*403200ull+tick;
    return true;
}
bool auth(const wire::Request *in,wire::Reply *out){
    out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce("");out->set_protocol_major(1);out->set_protocol_minor(7);
    out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
    if(in->client_nonce().size()<16||in->client_nonce().size()>64) return false;
    out->set_client_nonce(in->client_nonce());
    if(in->protocol_major()!=1||in->protocol_minor()!=7){out->set_failure_code(2);return false;}
    const char *configured=std::getenv("DFMCP_CONTROL_TOKEN");const std::string_view expected=configured?std::string_view(configured):std::string_view();
    const auto &presented=in->bearer_token();out->set_failure_code(1);if(expected.size()<32||expected.size()>256||presented.size()<32||presented.size()>256)return false;
    std::size_t diff=expected.size()^presented.size();for(std::size_t i=0;i<256;++i){unsigned char a=i<expected.size()?expected[i]:0,b=i<presented.size()?presented[i]:0;diff|=a^b;}if(diff)return false;
    out->set_failure_code(5);const auto &v=Core::getInstance().vinfo;const std::string df=v?v->getVersion():std::string(),dfhack=Version::dfhack_version();
    if(!generation||generation==std::numeric_limits<std::uint64_t>::max()||df.empty()||dfhack.empty())return false;
    out->set_bridge_generation(generation);out->set_df_version(df);out->set_dfhack_version(dfhack);out->set_failure_code(0);return true;
}
void fill_record(wire::Reply *out,const Record &r){
    out->set_effect_known(true);out->set_effect_applied(r.applied);out->set_paused(r.paused);out->set_observed_game_tick(r.tick);
    // A known prepare without a receipt is NOT a terminal not-applied result.
    if(!r.receipt.empty()) out->set_receipt_digest(r.receipt);
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out){if(auth(in,out))out->set_accepted(true);return CR_OK;}
command_result PreparePause(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!auth(in,out)) return CR_OK;
    out->set_failure_code(3);
    if(!Core::getInstance().isWorldLoaded()||!World::isFortressMode()){out->set_failure_code(4);return CR_OK;}
    if(!text_ok(in->idempotency_key(),512)||in->plan_digest().size()!=32||!in->has_paused()||!in->has_expected_game_tick())return CR_OK;
    auto found=records.find(in->idempotency_key());
    if(found!=records.end()){
        const auto &r=found->second;
        if(r.digest!=in->plan_digest()||r.desired_paused!=in->paused()||r.expected_tick!=in->expected_game_tick()){out->set_failure_code(7);return CR_OK;}
        out->set_prepare_token(r.token);fill_record(out,r);out->set_accepted(true);out->set_failure_code(0);return CR_OK;
    }
    std::uint64_t now=0;
    if(!clock_tick(now)){out->set_failure_code(5);return CR_OK;}
    if(now!=in->expected_game_tick()){out->set_failure_code(6);return CR_OK;}
    if(records.size()>=MAX_RECORDS)return CR_OK;
    Record r;r.digest=in->plan_digest();r.desired_paused=in->paused();r.expected_tick=now;
    r.paused=World::ReadPauseState();r.tick=now;r.token=hash_token(in->idempotency_key(),r.digest,now,r.desired_paused);
    records.emplace(in->idempotency_key(),r);out->set_prepare_token(r.token);out->set_effect_known(false);out->set_accepted(true);out->set_failure_code(0);return CR_OK;
}
command_result CommitPause(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!auth(in,out)) return CR_OK;
    out->set_failure_code(3);
    if(!text_ok(in->idempotency_key(),512)||in->plan_digest().size()!=32||in->prepare_token().size()!=16)return CR_OK;
    auto found=records.find(in->idempotency_key());if(found==records.end()||found->second.digest!=in->plan_digest()||found->second.token!=in->prepare_token()){out->set_failure_code(7);return CR_OK;}
    auto &r=found->second;
    if(r.requested){fill_record(out,r);out->set_accepted(true);out->set_failure_code(r.applied?0:5);return CR_OK;}
    if(!Core::getInstance().isWorldLoaded()||!World::isFortressMode()){out->set_failure_code(4);return CR_OK;}
    std::uint64_t now=0;
    if(!clock_tick(now)){out->set_failure_code(5);return CR_OK;}
    if(now<r.expected_tick){out->set_failure_code(6);return CR_OK;}
    // Mark before the setter: even an exception or lost response must not permit redispatch.
    r.requested=true;
    World::SetPauseState(r.desired_paused);
    r.paused=World::ReadPauseState();
    if(!clock_tick(now)||now<r.expected_tick){
        // The setter ran but a complete observation could not be established.
        // No terminal receipt is invented; a query must remain indeterminate.
        r.applied=false;r.receipt.clear();
    }else{
        r.tick=now;r.applied=r.paused==r.desired_paused;
        r.receipt=receipt(in->idempotency_key(),r.digest,r.desired_paused,r.tick);
    }
    fill_record(out,r);out->set_accepted(true);out->set_failure_code(r.applied?0:5);return CR_OK;
}
command_result QueryPause(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!auth(in,out)) return CR_OK;
    out->set_failure_code(3);
    if(!text_ok(in->idempotency_key(),512)||in->plan_digest().size()!=32)return CR_OK;
    auto found=records.find(in->idempotency_key());if(found==records.end()){out->set_effect_known(false);out->set_accepted(true);out->set_failure_code(0);return CR_OK;}
    if(found->second.digest!=in->plan_digest()){out->set_failure_code(7);return CR_OK;}fill_record(out,found->second);out->set_prepare_token(found->second.token);out->set_accepted(true);out->set_failure_code(0);return CR_OK;
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand>&){return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &){return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event){if((event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED)&&generation!=std::numeric_limits<std::uint64_t>::max()){++generation;records.clear();}return CR_OK;}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("PreparePause",PreparePause,0);s->addFunction("CommitPause",CommitPause,0);s->addFunction("QueryPause",QueryPause,0);return s;}
