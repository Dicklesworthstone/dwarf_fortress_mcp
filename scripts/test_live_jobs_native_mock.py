#!/usr/bin/env python3
"""Compile the actual jobs plugin against mock DFHack/protobuf types, not a DF build."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'bridge/dfhack-jobs-v1_2/dfmcp_jobs_v1_2.cpp'
STUBS = r'''
#pragma once
#include <cstdint>
#include <string>
#include <vector>
#include <memory>
namespace df {
struct unit { int32_t id=0; };
struct building { int32_t id=0; };
struct coord { int32_t x=1,y=2,z=3; };
struct job {
 int32_t id=0,job_type=5,completion_timer=-1;
 std::string reaction_name;
 struct { struct { bool suspend=true,repeat=false; } bits; } flags;
 coord pos;
 std::vector<void*> items,general_refs;
 struct {std::vector<void*> elements;} job_items;
 unit *worker=nullptr; building *holder=nullptr;
};
struct job_list_link {job *item=nullptr;job_list_link *next=nullptr;};
struct world {struct {job_list_link list;} jobs;};
namespace global {inline df::world *world=nullptr;inline int32_t *job_next_id=nullptr;}
}
inline std::string mock_job_key(int id){return id==5?"Dig":"CustomReaction";}
#define ENUM_KEY_STR(type,value) mock_job_key(static_cast<int>(value))
#define DFHACK_PLUGIN(name)
#define DFhackCExport extern "C"
namespace DFHack {
struct color_ostream{};struct PluginCommand{};
enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo{std::string value="test-df";std::string getVersion(){return value;}};
struct Core {bool loaded=true;std::shared_ptr<VersionInfo> vinfo=std::make_shared<VersionInfo>();
 static Core &getInstance(){static Core instance;return instance;}
 bool isWorldLoaded(){return loaded;}};
namespace Version {inline std::string dfhack_version(){return "test-dfhack";}}
namespace World {
 inline uint32_t year=105,tick=3;inline int32_t site=1;inline bool fortress=true,paused=true;
 inline std::string folder="region1";
 inline bool isFortressMode(){return fortress;}
 inline uint32_t ReadCurrentYear(){return year;}
 inline uint32_t ReadCurrentTick(){return tick;}
 inline int32_t GetCurrentSiteId(){return site;}
 inline bool ReadPauseState(){return paused;}
 inline std::string ReadWorldFolder(){return folder;}
}
namespace Job {inline df::unit *getWorker(df::job *j){return j->worker;}
 inline df::building *getHolder(df::job *j){return j->holder;}}
struct RPCService {std::vector<std::string> names;std::vector<int> flags;
 template<class F> void addFunction(const char *name,F,int flag){names.push_back(name);flags.push_back(flag);}};
}
namespace dfmcp::jobs::v1_2 {
struct Request {
 std::string token=std::string(32,'t'),nonce=std::string(16,'n');uint32_t major=1,minor=2,maximum=4;
 const std::string &bearer_token()const{return token;}
 const std::string &client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 uint32_t max_jobs()const{return maximum;}
};
struct Reply {
 bool accepted=false,has_observation=false;uint32_t failure_code=99,major=0,minor=0;
 uint64_t generation=0;std::string nonce,df_version,dfhack_version,observation;
 void Clear(){*this=Reply();}
 void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){failure_code=v;}
 void set_client_nonce(const std::string &v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}
 void set_protocol_minor(uint32_t v){minor=v;}void set_bridge_generation(uint64_t v){generation=v;}
 void set_df_version(const std::string &v){df_version=v;}void set_dfhack_version(const std::string &v){dfhack_version=v;}
 void set_observation(const std::string &v){observation=v;has_observation=true;}
};
}
'''
HARNESS = r'''
#include "PLUGIN_SOURCE"
#include <iostream>
#include <iomanip>
#include <stdexcept>
int checks=0;
void check(bool yes){++checks;if(!yes)throw std::runtime_error("check "+std::to_string(checks));}
int main(){
 setenv("DFMCP_JOBS_TOKEN",std::string(32,'t').c_str(),1);
 DFHack::color_ostream out;wire::Request request;wire::Reply reply;
 df::world world;int32_t next=10;df::global::world=&world;df::global::job_next_id=&next;
 df::job a,b;b.id=2;b.job_type=6;b.reaction_name="MAKE_STEEL";b.flags.bits.suspend=false;b.flags.bits.repeat=true;
 df::unit worker;worker.id=7;b.worker=&worker;df::building holder;holder.id=4;a.holder=&holder;
 df::job_list_link first{&b,nullptr},second{&a,nullptr};first.next=&second;world.jobs.list.next=&first;
 auto read=[&](){ReadObservation(out,&request,&reply);};
 auto rejected=[&](uint32_t code){read();check(!reply.accepted);check(reply.failure_code==code);
   check(!reply.has_observation);check(reply.observation.empty());};
 Handshake(out,&request,&reply);check(reply.accepted&&!reply.has_observation);
 request.token=std::string(32,'x');rejected(1);check(reply.df_version.empty()&&reply.generation==0);
 request.token=std::string(32,'t');request.minor=1;rejected(2);request.minor=2;
 request.maximum=0;rejected(3);request.maximum=1;rejected(3);request.maximum=4097;rejected(3);request.maximum=4;
 request.nonce="bad";rejected(3);request.nonce=std::string(16,'n');
 World::fortress=false;rejected(4);World::fortress=true;
 Core::getInstance().loaded=false;rejected(4);Core::getInstance().loaded=true;
 World::tick=403200;rejected(5);World::tick=3;
 World::folder=std::string("r\0x",3);rejected(5);World::folder="region1";
 b.id=0;rejected(5);b.id=2;
 b.id=10;rejected(5);b.id=2;
 b.completion_timer=-2;rejected(5);b.completion_timer=-1;
 b.reaction_name=std::string(129,'x');rejected(5);b.reaction_name="MAKE_STEEL";
 b.reaction_name=std::string("\xC0\x80",2);rejected(5);b.reaction_name="MAKE_STEEL";
 b.general_refs.push_back(nullptr);rejected(5);b.general_refs.clear();
 b.items.resize(65537);rejected(5);b.items.clear();
 b.job_items.elements.resize(4097);rejected(5);b.job_items.elements.clear();
 second.next=&first;rejected(3);second.next=nullptr;
 worker.id=-1;rejected(5);worker.id=7;
 read();check(reply.accepted&&reply.failure_code==0&&reply.has_observation);
 check(a.flags.bits.suspend&&!b.flags.bits.suspend&&World::paused);
 const auto golden=reply.observation;read();check(reply.observation==golden);
 auto old=generation;plugin_onstatechange(out,SC_WORLD_LOADED);check(generation==old+1);
 plugin_onstatechange(out,SC_OTHER);check(generation==old+1);
 read();check(reply.accepted&&reply.generation==generation);
 world.jobs.list.next=nullptr;read();check(reply.accepted&&reply.observation.size()<golden.size());
 auto *rpc=plugin_rpcconnect(out);check(rpc->names==std::vector<std::string>({"Handshake","ReadObservation"}));
 check(rpc->flags==std::vector<int>({0,0}));delete rpc;
 generation=std::numeric_limits<uint64_t>::max();rejected(5);
 for(unsigned char byte:golden)std::cout<<std::hex<<std::setw(2)<<std::setfill('0')<<int(byte);
 std::cout<<"\n"<<std::dec<<checks<<"\n";
}
'''

def expected_payload():
    out = bytearray(b'DFMJ1200')
    def u32(value): out.extend(struct.pack('>I', value))
    def i32(value): out.extend(struct.pack('>i', value))
    def text(value):
        value = value.encode(); out.extend(struct.pack('>H', len(value))); out.extend(value)
    u32(105); u32(3); out.append(1); i32(1); u32(10); text('region1'); u32(2)
    for identity, kind, name, reaction, suspend, repeat, worker, holder in [
        (0, 5, 'Dig', '', 1, 0, None, 4),
        (2, 6, 'CustomReaction', 'MAKE_STEEL', 0, 1, 7, None),
    ]:
        u32(identity); i32(kind); text(name); text(reaction); out.extend([suspend, repeat])
        for coordinate in (1, 2, 3): i32(coordinate)
        for reference in (worker, holder):
            out.append(reference is not None)
            if reference is not None: u32(reference)
        i32(-1); u32(0); u32(0)
    return bytes(out)

def main():
    compiler = shutil.which(os.environ.get('CXX', 'c++'))
    if compiler is None: raise SystemExit('C++ compiler required; no test was run')
    with tempfile.TemporaryDirectory(prefix='dfmcp-jobs-mock-') as temporary:
        root = Path(temporary)
        (root / 'mock.h').write_text(STUBS)
        headers = ['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h',
            'MiscUtils.h','DfmcpJobsV1_2.pb.h','modules/Job.h','modules/World.h',
            'df/building.h','df/global_objects.h','df/job.h','df/job_list_link.h','df/unit.h','df/world.h']
        for name in headers:
            path = root / name; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('#include "mock.h"\n')
        harness = root / 'test.cpp'
        harness.write_text(HARNESS.replace('PLUGIN_SOURCE', SOURCE.as_posix()))
        subprocess.run([compiler,'-std=c++17','-Wall','-Wextra','-Werror','-I',str(root),str(harness),'-o',str(root/'test')], check=True, timeout=30)
        result = subprocess.run([str(root/'test')], text=True, capture_output=True, check=True, timeout=10)
        golden, checks = result.stdout.strip().splitlines()
        actual = bytes.fromhex(golden)
        assert actual == expected_payload(), 'native serializer differs from independent golden payload'
        report = {'evidence':'native-source-with-mock-DFHack-and-protobuf-types',
            'native_dfhack_build':False,'rust_execution':False,'assertions':int(checks),
            'golden_bytes':len(actual),'source_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
            'golden_hex':golden}
        print(json.dumps(report, sort_keys=True))

if __name__ == '__main__': main()
