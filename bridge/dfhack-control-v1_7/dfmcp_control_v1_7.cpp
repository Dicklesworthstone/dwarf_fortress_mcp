#include "../common/retained_snapshot.h"
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <map>
#include <string>
#include <string_view>
#include <vector>
#include <utility>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/World.h"
#include "DfmcpControlV1_7.pb.h"
// In-process control authority only. The DFHack caller serializes access under
// CoreSuspender. This does not fence other plugins, UI input, or remote hosts.
namespace dfmcp_pause {

class DispatchFence {
public:
    using Clock = std::chrono::steady_clock;
    static constexpr std::chrono::seconds PREPARE_LIFETIME{60};
    enum class State { Pending, Claimed, Retired };
    struct Preparation {
        std::uint64_t sequence;
        std::uint64_t game_tick;
        bool observed_paused;
        Clock::time_point created;
        State state = State::Pending;
    };

    bool available() const { return sequence != std::numeric_limits<std::uint64_t>::max(); }

    Preparation prepare(std::uint64_t tick, bool paused, Clock::time_point now) const {
        return {sequence, tick, paused, now, available() ? State::Pending : State::Retired};
    }

    // Called immediately before every potentially effectful setter, including a
    // no-op setter. Increment *before* the effect, not after its acknowledgement.
    // A failed or ambiguous setter must invalidate competing old preparations too.
    bool claim(Preparation &preparation, std::uint64_t tick, bool paused, Clock::time_point now) {
        if (preparation.state != State::Pending) return false;
        // Compare a bounded deadline instead of subtracting arbitrary time
        // points (which can overflow the clock's signed duration representation).
        const auto latest_start = Clock::time_point::max() - PREPARE_LIFETIME;
        const bool fresh = now >= preparation.created &&
            (preparation.created > latest_start || now < preparation.created + PREPARE_LIFETIME);
        if (!available() || preparation.sequence != sequence || tick < preparation.game_tick
            || paused != preparation.observed_paused || !fresh) {
            preparation.state = State::Retired;
            return false;
        }
        ++sequence;
        preparation.state = State::Claimed;
        return true;
    }

    void retire(Preparation &preparation) const {
        if (preparation.state == State::Pending) preparation.state = State::Retired;
    }

private:
    std::uint64_t sequence = 0;
};

} // namespace dfmcp_pause

using namespace DFHack;
namespace wire=dfmcp::control::v1_7;
DFHACK_PLUGIN("dfmcp_control_v1_7");
namespace {
constexpr std::size_t MAX_RECORDS=4096;
dfmcp_pause::DispatchFence dispatch_fence;
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|1;
struct Record {
    // Preparation identity must survive a later observation at another game tick.
    std::string digest,token,receipt;
    bool desired_paused=false;
    std::uint64_t expected_tick=0;
    bool requested=false,applied=false,paused=false;
    std::uint64_t tick=0;
    dfmcp_pause::DispatchFence::Preparation guard{};
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
    if(in->query_only()||!text_ok(in->idempotency_key(),512)||in->plan_digest().size()!=32||!in->has_paused()||!in->has_expected_game_tick())return CR_OK;
    auto found=records.find(in->idempotency_key());
    if(found!=records.end()){
        const auto &r=found->second;
        if(r.digest!=in->plan_digest()||r.desired_paused!=in->paused()||r.expected_tick!=in->expected_game_tick()){out->set_failure_code(7);return CR_OK;}
        out->set_prepare_token(r.token);fill_record(out,r);out->set_accepted(true);out->set_failure_code(0);return CR_OK;
    }
    std::uint64_t now=0;
    if(!clock_tick(now)){out->set_failure_code(5);return CR_OK;}
    if(now!=in->expected_game_tick()){out->set_failure_code(6);return CR_OK;}
    if(records.size()>=MAX_RECORDS||!dispatch_fence.available())return CR_OK;
    Record r;r.digest=in->plan_digest();r.desired_paused=in->paused();r.expected_tick=now;
    r.paused=World::ReadPauseState();r.tick=now;r.token=hash_token(in->idempotency_key(),r.digest,now,r.desired_paused);
    r.guard=dispatch_fence.prepare(now,r.paused,dfmcp_pause::DispatchFence::Clock::now());
    records.emplace(in->idempotency_key(),r);out->set_prepare_token(r.token);out->set_effect_known(false);out->set_accepted(true);out->set_failure_code(0);return CR_OK;
}
command_result CommitPause(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!auth(in,out)) return CR_OK;
    out->set_failure_code(3);
    if(in->query_only()||!text_ok(in->idempotency_key(),512)||in->plan_digest().size()!=32||in->prepare_token().size()!=16)return CR_OK;
    auto found=records.find(in->idempotency_key());if(found==records.end()||found->second.digest!=in->plan_digest()||found->second.token!=in->prepare_token()){out->set_failure_code(7);return CR_OK;}
    auto &r=found->second;
    if(r.requested){fill_record(out,r);out->set_accepted(true);out->set_failure_code(r.applied?0:5);return CR_OK;}
    if(!Core::getInstance().isWorldLoaded()||!World::isFortressMode()){
        dispatch_fence.retire(r.guard);out->set_failure_code(4);return CR_OK;
    }
    std::uint64_t now=0;
    try {
        if(!clock_tick(now)){
            dispatch_fence.retire(r.guard);out->set_failure_code(5);return CR_OK;
        }
        // All preparations preceding another setter attempt are fenced, even
        // if pause state later returns to its original value at the same tick.
        if(!dispatch_fence.claim(r.guard,now,World::ReadPauseState(),dfmcp_pause::DispatchFence::Clock::now())){
            out->set_failure_code(6);return CR_OK;
        }
        // Mark and fence before the setter. An exception must never redispatch.
        r.requested=true;
        World::SetPauseState(r.desired_paused);
        const bool paused=World::ReadPauseState();
        if(!clock_tick(now)||now<r.expected_tick){
            r.applied=false;r.receipt.clear();
        }else{
            // Construct terminal evidence first, then publish the record fields.
            auto proof=receipt(in->idempotency_key(),r.digest,r.desired_paused,now);
            r.tick=now;r.paused=paused;r.applied=paused==r.desired_paused;
            r.receipt=std::move(proof);
        }
    } catch (...) {
        // No exception from a setter/readback may escape the RPC boundary and
        // terminate its thread/process. A receipt-less known record is unknown,
        // not verified non-application. Pre-setter failures retire the guard.
        dispatch_fence.retire(r.guard);r.applied=false;r.receipt.clear();
        out->set_failure_code(5);
        if(!r.requested)return CR_OK;
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
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event){
    // A fortress/map can change while the world remains loaded. Never carry
    // prepared tokens or receipt lookups into that new local map incarnation.
    if(event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED||event==SC_MAP_LOADED||event==SC_MAP_UNLOADED){
        if(generation!=std::numeric_limits<std::uint64_t>::max())++generation;
        records.clear();
    }
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("PreparePause",PreparePause,0);s->addFunction("CommitPause",CommitPause,0);s->addFunction("QueryPause",QueryPause,0);return s;}
