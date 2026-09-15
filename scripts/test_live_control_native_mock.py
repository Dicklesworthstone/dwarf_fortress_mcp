#!/usr/bin/env python3
"""Compile the actual control/1.7 producer against mocks; never native qualification."""
import argparse, hashlib, json, pathlib, subprocess, tempfile

MOCK = r'''
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace DFHack {
struct color_ostream{}; struct PluginCommand{}; enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo{std::string getVersion(){return "df";}};
struct Core{std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();bool loaded=true;
 static Core& getInstance(){static Core c;return c;}bool isWorldLoaded(){return loaded;}};
namespace Version{inline std::string dfhack_version(){return "dfhack";}}
namespace World{inline int64_t year=105;inline uint32_t tick=7;inline bool paused=false,fortress=true;inline int set_calls=0;
 inline int64_t ReadCurrentYear(){return year;}inline uint32_t ReadCurrentTick(){return tick;}
 inline bool ReadPauseState(){return paused;}inline void SetPauseState(bool v){++set_calls;paused=v;}
 inline bool isFortressMode(){return fortress;}}
struct RPCService{std::vector<std::string>names;std::vector<int>flags;
 template<class F>void addFunction(const char*n,F,int f){names.push_back(n);flags.push_back(f);}};
}
namespace dfmcp::control::v1_7 {
struct Request{
 std::string token=std::string(32,'t'),nonce=std::string(16,'n'),key,digest,prepare;
 uint32_t major=1,minor=7;bool paused_value=false,paused_present=false,tick_present=false,query=false;uint64_t tick=0;
 const std::string& bearer_token()const{return token;}const std::string& client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 const std::string& idempotency_key()const{return key;}const std::string& plan_digest()const{return digest;}
 const std::string& prepare_token()const{return prepare;}bool has_paused()const{return paused_present;}bool paused()const{return paused_value;}
 bool has_expected_game_tick()const{return tick_present;}uint64_t expected_game_tick()const{return tick;}bool query_only()const{return query;}
};
struct Reply{
 bool accepted=false,known=false,applied=false,paused=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0,observed=0;
 std::string nonce,df,dfhack,prepare,receipt;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string&v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}void set_protocol_minor(uint32_t v){minor=v;}
 void set_bridge_generation(uint64_t v){generation=v;}void set_df_version(const std::string&v){df=v;}void set_dfhack_version(const std::string&v){dfhack=v;}
 void set_prepare_token(const std::string&v){prepare=v;}void set_effect_known(bool v){known=v;}void set_effect_applied(bool v){applied=v;}
 void set_paused(bool v){paused=v;}void set_observed_game_tick(uint64_t v){observed=v;}void set_receipt_digest(const std::string&v){receipt=v;}
};
}
'''

DRIVER = r'''
#include <iomanip>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include "PRODUCER"
int checks=0;void check(bool c){++checks;if(!c)throw std::runtime_error("check "+std::to_string(checks));}
std::string hex(const std::string&v){std::ostringstream s;for(unsigned char c:v)s<<std::hex<<std::setw(2)<<std::setfill('0')<<int(c);return s.str();}
int main(){setenv("DFMCP_CONTROL_TOKEN",std::string(32,'t').c_str(),1);color_ostream out;wire::Request request;wire::Reply reply;
 Handshake(out,&request,&reply);check(reply.accepted);check(reply.code==0);check(reply.minor==7);check(reply.prepare.empty());check(World::set_calls==0);
 request.key="pause-1";request.digest=std::string(32,'d');request.paused_present=true;request.paused_value=true;request.tick_present=true;
 request.tick=uint64_t(World::year)*403200ull+World::tick;
 PreparePause(out,&request,&reply);check(reply.accepted);check(reply.code==0);check(reply.prepare.size()==16);check(!reply.known);const auto token1=reply.prepare;
 PreparePause(out,&request,&reply);check(reply.accepted);check(reply.prepare==token1);check(World::set_calls==0);
 request.paused_value=false;PreparePause(out,&request,&reply);check(!reply.accepted);check(reply.code==7);request.paused_value=true;
 request.digest=std::string(32,'x');PreparePause(out,&request,&reply);check(!reply.accepted);check(reply.code==7);request.digest=std::string(32,'d');
 request.prepare=std::string(16,'z');CommitPause(out,&request,&reply);check(!reply.accepted);check(reply.code==7);check(World::set_calls==0);
 request.prepare=token1;CommitPause(out,&request,&reply);check(reply.accepted);check(reply.code==0);check(reply.known);check(reply.applied);check(reply.paused);
 check(reply.receipt.size()==32);check(World::set_calls==1);const auto receipt1=reply.receipt;const auto observed=reply.observed;
 CommitPause(out,&request,&reply);check(reply.accepted);check(reply.receipt==receipt1);check(reply.observed==observed);check(World::set_calls==1);
 QueryPause(out,&request,&reply);check(reply.accepted);check(reply.known);check(reply.applied);check(reply.receipt==receipt1);check(reply.prepare==token1);
 request.key="unknown";QueryPause(out,&request,&reply);check(reply.accepted);check(!reply.known);check(reply.receipt.empty());request.key="pause-1";
 const auto old_generation=generation;plugin_onstatechange(out,SC_WORLD_UNLOADED);check(generation==old_generation+1);QueryPause(out,&request,&reply);check(reply.accepted);check(!reply.known);
 World::paused=false;World::set_calls=0;PreparePause(out,&request,&reply);check(reply.accepted);check(reply.prepare.size()==16);check(reply.prepare!=token1);const auto token2=reply.prepare;
 request.prepare=token1;CommitPause(out,&request,&reply);check(!reply.accepted);check(reply.code==7);check(World::set_calls==0);
 request.prepare=token2;CommitPause(out,&request,&reply);check(reply.accepted);check(reply.applied);check(World::set_calls==1);check(reply.receipt.size()==32);
 request.token="short";Handshake(out,&request,&reply);check(!reply.accepted);request.token=std::string(32,'t');request.minor=6;Handshake(out,&request,&reply);check(!reply.accepted);check(reply.code==2);request.minor=7;
 request.nonce="short";Handshake(out,&request,&reply);check(!reply.accepted);request.nonce=std::string(16,'n');Core::getInstance().loaded=false;
 request.key="new";PreparePause(out,&request,&reply);check(!reply.accepted);check(reply.code==4);Core::getInstance().loaded=true;World::fortress=false;PreparePause(out,&request,&reply);check(!reply.accepted);check(reply.code==4);World::fortress=true;
 auto*service=plugin_rpcconnect(out);check(service->names==std::vector<std::string>({"Handshake","PreparePause","CommitPause","QueryPause"}));check(service->flags==std::vector<int>({0,0,0,0}));delete service;
 std::cout<<"{\"checks\":"<<checks<<",\"generation1\":"<<old_generation<<",\"generation2\":"<<generation
          <<",\"token1\":\""<<hex(token1)<<"\",\"token2\":\""<<hex(token2)<<"\",\"receipt1\":\""<<hex(receipt1)<<"\"}\n";
}
'''

def expected_token(generation, key, digest, tick, paused):
    data=b'dfmcp-control-token-v2\0'+generation.to_bytes(8,'big')+key.encode()+b'\0'+digest+tick.to_bytes(8,'big')+bytes([paused])
    return hashlib.sha256(data).digest()[:16]

def expected_receipt(generation, key, digest, paused, tick):
    data=b'dfmcp-control-receipt-v2\0'+generation.to_bytes(8,'big')+key.encode()+b'\0'+digest+bytes([paused])+tick.to_bytes(8,'big')
    return hashlib.sha256(data).digest()

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1];producer=root/'bridge/dfhack-control-v1_7/dfmcp_control_v1_7.cpp'
    with tempfile.TemporaryDirectory(prefix='dfmcp-control-mock-') as tmp:
        tmp=pathlib.Path(tmp);(tmp/'mock.hpp').write_text(MOCK)
        for name in ['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','modules/World.h','DfmcpControlV1_7.pb.h']:
            path=tmp/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (tmp/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(producer)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic']
        subprocess.run([args.compiler,*flags,'-I',str(tmp),str(tmp/'test.cpp'),'-o',str(tmp/'test')],check=True,timeout=60)
        result=json.loads(subprocess.run([str(tmp/'test')],capture_output=True,text=True,check=True,timeout=15).stdout)
        tick=105*403200+7;digest=b'd'*32
        assert result['token1']==expected_token(result['generation1'],'pause-1',digest,tick,True).hex()
        assert result['token2']==expected_token(result['generation2'],'pause-1',digest,tick,True).hex()
        assert result['receipt1']==expected_receipt(result['generation1'],'pause-1',digest,True,tick).hex()
        result.update(compiler=args.compiler,flags=flags,source_sha256=hashlib.sha256(producer.read_bytes()).hexdigest(),
            evidence='mock_interfaces_not_real_dfhack_or_rust_execution')
        if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result,indent=2))
if __name__=='__main__':main()
