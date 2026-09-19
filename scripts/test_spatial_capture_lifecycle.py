#!/usr/bin/env python3
"""Execute the real spatial/1.8 RPC handlers and retained cache with boundary doubles.

Native field capture and generated protobuf/DFHack interfaces are explicit mocks.
This is not a full plugin build, Rust execution or live-game qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

MOCK = r'''
#pragma once
#include <cstdint>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace faults {
inline int identity=0,capture=0,reply=0;inline unsigned captures=0;inline bool null_version=false;
inline std::string payload(40000,'p');
inline void fail(int mode){if(mode==1)throw std::runtime_error("private native details");if(mode==2)throw 7;}
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};
enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_MAP_LOADED,SC_MAP_UNLOADED,SC_OTHER};
struct VersionInfo{std::string getVersion(){faults::fail(faults::identity);return "df-fixture";}};
struct Core{std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();
 static Core&getInstance(){static Core c;return c;}};
namespace Version{inline const char* dfhack_version(){return faults::null_version?nullptr:"dfhack-fixture";}}
struct RPCService{std::vector<std::string>names;std::vector<int>flags;
 template<class F>void addFunction(const char*name,F,int flag){names.push_back(name);flags.push_back(flag);}};
}
namespace dfmcp_spatial_capture {
struct Bounds{uint32_t jobs,buildings,items,bytes;uint32_t origin[3],size[3];
 bool valid()const{return jobs&&buildings&&items&&bytes>=1024&&bytes<=16*1024*1024&&size[0]&&size[1]&&size[2];}};
inline void u32(std::string&out,uint32_t n){for(int s=24;s>=0;s-=8)out.push_back(static_cast<char>(n>>s));}
inline void text(std::string&out,const std::string&s){out.push_back(static_cast<char>(s.size()>>8));out.push_back(static_cast<char>(s.size()));out+=s;}
inline bool utf8(const std::string&s,size_t bound){return !s.empty()&&s.size()<=bound&&s.find('\0')==std::string::npos;}
}
namespace dfmcp_spatial_citizen_capture {
struct Bounds{dfmcp_spatial_capture::Bounds spatial_bounds;uint32_t max_citizens;
 bool valid()const{return spatial_bounds.valid()&&max_citizens&&max_citizens<=4096;}};
inline uint32_t capture(const Bounds&,std::string&out){++faults::captures;faults::fail(faults::capture);out=faults::payload;return 0;}
}
namespace dfmcp::spatial::v1_8 {
struct Request {
 std::string token=std::string(32,'t'),nonce=std::string(16,'n'),snapshot;
 uint32_t major=1,minor=8,jobs=4096,buildings=4096,items=65536,bytes=16*1024*1024,page=16384,citizens=4096;
 uint32_t ox=0,oy=0,oz=5,sx=4,sy=4,sz=1;uint64_t off=0;bool releasing=false;
 const std::string&bearer_token()const{return token;}const std::string&client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 uint32_t max_jobs()const{return jobs;}uint32_t max_buildings()const{return buildings;}uint32_t max_items()const{return items;}
 uint32_t max_bytes()const{return bytes;}uint32_t page_bytes()const{return page;}uint32_t max_citizens()const{return citizens;}
 uint64_t offset()const{return off;}const std::string&snapshot_token()const{return snapshot;}bool release()const{return releasing;}
 uint32_t x()const{return ox;}uint32_t y()const{return oy;}uint32_t z()const{return oz;}
 uint32_t width()const{return sx;}uint32_t height()const{return sy;}uint32_t depth()const{return sz;}
};
struct Reply {
 bool accepted=false,has_payload=false,complete=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0,offset=0,total=0;
 std::string nonce,df,dfhack,payload,token,digest;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string&v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}void set_protocol_minor(uint32_t v){minor=v;}
 void set_bridge_generation(uint64_t v){generation=v;}void set_df_version(const std::string&v){df=v;}void set_dfhack_version(const std::string&v){dfhack=v;}
 void set_observation(const std::string&v){payload=v;has_payload=true;if(faults::reply==1){faults::reply=0;throw std::runtime_error("reply copy failed");}}
 void set_snapshot_token(const std::string&v){token=v;if(faults::reply==2){faults::reply=0;throw 7;}}
 void set_page_offset(uint64_t v){offset=v;}void set_total_bytes(uint64_t v){total=v;}
 void set_payload_sha256(const std::string&v){digest=v;}void set_complete(bool v){complete=v;if(faults::reply==3){faults::reply=0;throw std::runtime_error("late reply failed");}}
};
}
'''

ALLOCATOR = r'''
#include <cstddef>
#include <cstdlib>
#include <new>
namespace allocations {bool enabled=false;long remaining=0;}
void* operator new(std::size_t n) {
    if(allocations::enabled) {
        if(allocations::remaining==0)throw std::bad_alloc();
        --allocations::remaining;
    }
    if(void*p=std::malloc(n?n:1))return p;
    throw std::bad_alloc();
}
void operator delete(void*p)noexcept {std::free(p);}
void operator delete(void*p,std::size_t)noexcept {std::free(p);}
void* operator new[](std::size_t n) {return ::operator new(n);}
void operator delete[](void*p)noexcept {::operator delete(p);}
void operator delete[](void*p,std::size_t)noexcept {::operator delete(p);}
'''

DRIVER = r'''
#include <cstdlib>
#include <iostream>
#include <new>
#include <stdexcept>
// Test allocator lives in a separate translation unit, without LTO. Compiler
// inlining must not replace the intentionally injected allocation boundary.
namespace allocations {extern bool enabled;extern long remaining;}
#include "bridge/dfhack-spatial-v1_8/dfmcp_spatial_v1_8.cpp"
unsigned checks=0,groups=0;std::string current;
void check(bool v,const char*reason){++checks;if(!v)throw std::runtime_error(reason);}
void no_payload(const wire::Reply&p){check(!p.accepted,"accepted failed reply");check(!p.has_payload&&p.payload.empty(),"partial payload escaped");
 check(p.token.empty()&&p.digest.empty()&&p.total==0,"partial identity escaped");}
int main(int argc,char**argv){try{
 setenv("DFMCP_SPATIAL_CITIZEN_TOKEN",std::string(32,'t').c_str(),1);color_ostream out;
 const std::string selected=argc>1?argv[1]:"";
 auto run=[&](const char*name,auto body){if(!selected.empty()&&selected!=name)return;current=name;++groups;
  snapshots.clear();generation=100;faults::identity=faults::capture=faults::reply=0;faults::captures=0;faults::null_version=false;body();};
 run("map_lifecycle",[&]{
  for(auto event:{SC_MAP_UNLOADED,SC_MAP_LOADED,SC_WORLD_UNLOADED,SC_WORLD_LOADED}){
   wire::Request r;wire::Reply p;ReadObservation(out,&r,&p);check(p.accepted,"initial capture");const auto token=p.token;const auto before=generation;
   check(snapshots.count()==1,"capture missing");plugin_onstatechange(out,event);
   check(generation==before+1,"incarnation did not advance");check(snapshots.count()==0&&snapshots.bytes()==0,"old capture survived map transition");
   r.snapshot=token;ReadObservation(out,&r,&p);no_payload(p);check(p.code==6,"old token accepted after transition");
   r.snapshot.clear();ReadObservation(out,&r,&p);check(p.accepted&&p.token!=token,"fresh token reused");snapshots.clear();
  }
  wire::Request r;wire::Reply p;ReadObservation(out,&r,&p);auto token=p.token;const auto before=generation;
  plugin_onstatechange(out,SC_OTHER);check(generation==before,"unrelated state reset");r.snapshot=token;ReadObservation(out,&r,&p);check(p.accepted,"unrelated event lost capture");
 });
 run("new_reply_failure",[&]{
  for(int mode:{1,2,3})for(int attempt=0;attempt<6;++attempt){
   wire::Request r;wire::Reply p;faults::reply=mode;ReadObservation(out,&r,&p);no_payload(p);
   check(snapshots.count()==0&&snapshots.bytes()==0,"failed first response stranded retained capture");
  }
  wire::Request r;wire::Reply p;ReadObservation(out,&r,&p);check(p.accepted,"cache capacity not recovered");
 });
 run("continuation_failure",[&]{
  wire::Request r;wire::Reply p;ReadObservation(out,&r,&p);check(p.accepted&&!p.complete,"first page");
  const auto token=p.token,digest=p.digest,first=p.payload;const auto captures=faults::captures;
  r.snapshot=token;r.off=first.size();
  for(int mode:{1,2,3}){faults::reply=mode;ReadObservation(out,&r,&p);no_payload(p);check(snapshots.count()==1,"acknowledged capture lost on failed continuation");
   ReadObservation(out,&r,&p);check(p.accepted&&p.token==token&&p.digest==digest,"continuation did not resume same capture");}
  check(faults::captures==captures,"continuation recaptured native state");
  std::string assembled=first;
  while(assembled.size()<faults::payload.size()){r.off=assembled.size();ReadObservation(out,&r,&p);check(p.accepted,"paged read");assembled+=p.payload;}
  check(assembled==faults::payload,"mixed or incomplete retained bytes");check(p.complete,"missing final page marker");
  r.releasing=true;r.off=0;ReadObservation(out,&r,&p);check(p.accepted&&snapshots.count()==0,"release did not free capture");
 });
 run("exceptions",[&]{
  for(int mode:{1,2}){
   wire::Request r;wire::Reply p;faults::identity=mode;Handshake(out,&r,&p);no_payload(p);check(p.generation==0&&p.df.empty(),"identity leaked on handshake failure");
   ReadObservation(out,&r,&p);no_payload(p);check(faults::captures==0,"identity failure acquired data");
   faults::identity=0;faults::capture=mode;ReadObservation(out,&r,&p);no_payload(p);check(snapshots.count()==0,"failed capture published");faults::capture=0;faults::captures=0;
  }
 });
 run("authority_and_bounds",[&]{
  wire::Request r;wire::Reply p;Handshake(out,&r,&p);check(p.accepted&&p.minor==8,"valid handshake");
  faults::null_version=true;Handshake(out,&r,&p);no_payload(p);check(p.code==5,"null version admitted");faults::null_version=false;
  r.token=std::string(32,'x');ReadObservation(out,&r,&p);no_payload(p);check(p.code==1&&faults::captures==0,"bad credential crossed boundary");r.token=std::string(32,'t');
  r.minor=6;ReadObservation(out,&r,&p);no_payload(p);check(p.code==2,"wrong protocol");r.minor=8;
  r.nonce="short";ReadObservation(out,&r,&p);no_payload(p);r.nonce=std::string(16,'n');
  ReadObservation(out,&r,&p);check(p.accepted,"capture for ownership test");r.snapshot=p.token;
  r.nonce=std::string(16,'z');ReadObservation(out,&r,&p);no_payload(p);r.nonce=std::string(16,'n');
  r.ox=1;ReadObservation(out,&r,&p);no_payload(p);r.ox=0;r.citizens=4095;ReadObservation(out,&r,&p);no_payload(p);r.citizens=4096;
  r.off=faults::payload.size();ReadObservation(out,&r,&p);no_payload(p);check(snapshots.count()==1,"bad offset removed acknowledged token");
  auto*service=plugin_rpcconnect(out);check(service->names==std::vector<std::string>({"Handshake","ReadObservation"}),"RPC surface widened");check(service->flags==std::vector<int>({0,0}),"RPC suspension flags changed");delete service;
 });
 run("generation_exhaustion",[&]{
  wire::Request r;wire::Reply p;ReadObservation(out,&r,&p);check(p.accepted,"capture before exhaustion");
  generation=std::numeric_limits<uint64_t>::max()-1;plugin_onstatechange(out,SC_MAP_UNLOADED);
  check(generation==std::numeric_limits<uint64_t>::max()&&snapshots.count()==0,"exhausted incarnation wrapped");
  Handshake(out,&r,&p);no_payload(p);plugin_onstatechange(out,SC_MAP_LOADED);check(generation==std::numeric_limits<uint64_t>::max(),"exhausted generation reused");
 });
 run("allocation_rollback",[&]{
  unsigned failures=0,successes=0;const std::string owner(32,'o');
  for(long failure=0;failure<40;++failure){
   dfmcp_snapshot::Cache cache;std::string payload(4096,'d'),token;dfmcp_snapshot::Limits limits{1,1,1,4096};
   bool threw=false,inserted=false;allocations::remaining=failure;allocations::enabled=true;
   try{inserted=cache.insert(owner,1,limits,std::move(payload),dfmcp_snapshot::Cache::Clock::time_point{},token);}catch(const std::bad_alloc&){threw=true;}
   allocations::enabled=false;
   if(threw){++failures;check(cache.count()==0&&cache.bytes()==0,"allocation failure published an unreachable capture");}
   else{++successes;check(inserted&&token.size()==16&&cache.count()==1&&cache.bytes()==4096,"successful cache insertion lost identity");}
  }
  check(failures>0&&successes>0,"allocation campaign did not reach both branches");
 });
 check(groups!=0,"unknown test group");const auto hash=dfmcp_snapshot::sha256(faults::payload);const char*digits="0123456789abcdef";std::string hex;
 for(unsigned char c:hash){hex+=digits[c>>4];hex+=digits[c&15];}
 std::cout<<"{\"checks\":"<<checks<<",\"groups\":"<<groups<<",\"payload_sha256\":\""<<hex<<"\"}\n";
 }catch(const std::exception&e){allocations::enabled=false;std::cerr<<"FAIL "<<current<<": "<<e.what()<<"\n";return 1;}
 catch(...){allocations::enabled=false;std::cerr<<"FAIL "<<current<<": exception escaped native handler\n";return 2;}}
'''


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--cache-source", type=Path)
    parser.add_argument("--group", default="")
    parser.add_argument("--ubsan", action="store_true")
    parser.add_argument("--legacy-version-double", action="store_true",
                        help="Reproduce old handler defects with its historical string-returning version double")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = args.source or root / "bridge/dfhack-spatial-v1_8/dfmcp_spatial_v1_8.cpp"
    cache = args.cache_source or root / "bridge/common/retained_snapshot.h"
    source_bytes, cache_bytes = source.read_bytes(), cache.read_bytes()
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O1"]
    if args.ubsan:
        flags += ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
    with tempfile.TemporaryDirectory(prefix="dfmcp-spatial-lifecycle-") as tmp:
        tmp = Path(tmp)
        producer = tmp / "bridge/dfhack-spatial-v1_8/dfmcp_spatial_v1_8.cpp"
        producer.parent.mkdir(parents=True)
        producer.write_bytes(source_bytes)
        common = tmp / "bridge/common"
        common.mkdir()
        (common / "retained_snapshot.h").write_bytes(cache_bytes)
        (common / "spatial_citizen_capture.h").write_text('#include "mock.hpp"\n')
        mock = MOCK
        if args.legacy_version_double:
            mock = mock.replace('inline const char* dfhack_version(){return faults::null_version?nullptr:"dfhack-fixture";}',
                                'inline std::string dfhack_version(){return "dfhack-fixture";}')
        (tmp / "mock.hpp").write_text(mock)
        for name in ["Core.h", "Export.h", "PluginManager.h", "RemoteServer.h", "VersionInfo.h", "DfmcpSpatialV1_8.pb.h"]:
            (tmp / name).write_text('#include "mock.hpp"\n')
        (tmp / "test.cpp").write_text(DRIVER)
        (tmp / "allocator.cpp").write_text(ALLOCATOR)
        executable = tmp / "test"
        subprocess.run([args.compiler, *flags, "-I", str(tmp), str(tmp / "test.cpp"), str(tmp / "allocator.cpp"), "-o", str(executable)],
                       check=True, timeout=60)
        result = subprocess.run([str(executable), args.group], capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise RuntimeError(result.stderr.strip() or f"native exit {result.returncode}")
        evidence = json.loads(result.stdout)
    if evidence["payload_sha256"] != hashlib.sha256(b"p" * 40000).hexdigest():
        raise AssertionError("native SHA-256 does not match independent Python digest")
    evidence.update(compiler=args.compiler, flags=flags, legacy_version_double=args.legacy_version_double,
                    source_sha256=hashlib.sha256(source_bytes).hexdigest(),
                    cache_sha256=hashlib.sha256(cache_bytes).hexdigest(),
                    evidence="Actual RPC/cache code with capture/DFHack/protobuf doubles; not Rust, full plugin or live qualification")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
