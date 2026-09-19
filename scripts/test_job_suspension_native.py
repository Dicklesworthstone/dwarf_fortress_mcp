#!/usr/bin/env python3
"""Compile the actual job-control plugin/engine with explicit DFHack/protobuf doubles.

This is not a real DFHack, protobuf runtime, Rust or live-game qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
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
#define ENUM_KEY_STR(kind,value) std::string("BrewDrink")
struct color_ostream {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_PAUSED, SC_UNPAUSED, SC_OTHER };
struct TrackedBool {
 bool value=false; inline static unsigned writes=0; inline static bool fail=false;
 operator bool() const { return value; }
 TrackedBool &operator=(bool v) { ++writes;value=v;if(fail)throw 42;return *this; }
};
namespace df {
enum class building_type { Workshop=1, Furnace=2, Other=3 };
struct building {
 int id=20,stage=3,maximum=3;building_type kind=building_type::Workshop;
 building_type getType() const {return kind;} int getBuildStage() const {return stage;} int getMaxBuildStage() const {return maximum;}
};
struct unit {int id=42;};
struct job {
 int id=3,job_type=5,completion_timer=-1;
 struct {int x=1,y=2,z=5;} pos;
 struct {struct {TrackedBool suspend;bool repeat=true;} bits;} flags;
 struct {std::vector<void*> elements;} job_items;
 std::vector<void*> general_refs,items;std::string reaction_name;
 building *holder=nullptr;unit *worker=nullptr;bool supported=true;
};
namespace global {inline int counter=10;inline int *job_next_id=&counter;}
}
namespace DFHack {
struct PluginCommand {};
struct VersionInfo {inline static int failure=0;std::string getVersion(){if(failure==1)throw 42;if(failure==2)throw std::runtime_error("private");return "df";}};
struct Core {
 bool loaded=true,map=true;std::shared_ptr<VersionInfo> vinfo=std::make_shared<VersionInfo>();
 static Core &getInstance(){static Core core;return core;}
 bool isWorldLoaded() const{return loaded;}bool isMapLoaded() const{return map;}
};
namespace Version {inline bool null_version=false;inline const char *dfhack_version(){return null_version?nullptr:"dfhack";}}
namespace World {
 inline int year=105,site=1;inline std::uint32_t tick=3;inline bool paused=true,fortress=true;inline std::string folder="region1";
 inline int ReadCurrentYear(){return year;}inline std::uint32_t ReadCurrentTick(){return tick;}
 inline int GetCurrentSiteId(){return site;}inline std::string ReadWorldFolder(){return folder;}
 inline bool ReadPauseState(){return paused;}inline bool isFortressMode(){return fortress;}
}
namespace Job {
 inline df::job *current=nullptr;inline bool fail_read=false,fail_after_write=false;
 inline df::job *getJob(int id){if(fail_read||(fail_after_write&&TrackedBool::writes))throw 42;return current&&current->id==id?current:nullptr;}
 inline df::unit *getWorker(df::job*j){return j->worker;}inline df::building *getHolder(df::job*j){return j->holder;}
 inline bool isSupportedJob(df::job*j){return j->supported;}
}
}
namespace dfmcp::job_control::v1_9 {
struct Request {
 std::string auth=std::string(32,'t'),nonce=std::string(16,'n'),key,witness,plan,token;
 unsigned major=1,minor=9,mask=0,id=3;bool desired=true;
 const std::string &bearer_token() const{return auth;}const std::string &client_nonce() const{return nonce;}
 unsigned protocol_major() const{return major;}unsigned protocol_minor() const{return minor;}
 bool has_idempotency_key() const{return mask&1;}const std::string &idempotency_key() const{return key;}
 bool has_native_job_id() const{return mask&2;}unsigned native_job_id() const{return id;}
 bool has_suspended() const{return mask&4;}bool suspended() const{return desired;}
 bool has_expected_witness() const{return mask&8;}const std::string &expected_witness() const{return witness;}
 bool has_plan_digest() const{return mask&16;}const std::string &plan_digest() const{return plan;}
 bool has_prepare_token() const{return mask&32;}const std::string &prepare_token() const{return token;}
};
struct Reply {
 bool accepted=false,replayed=false;unsigned code=0,major=0,minor=0;std::uint64_t generation=0;
 std::string nonce,df,dfhack,observation,record;inline static bool fail_success=false;
 void Clear(){accepted=false;replayed=false;code=0;major=0;minor=0;generation=0;nonce.clear();df.clear();dfhack.clear();observation.clear();record.clear();}
 void set_accepted(bool b){if(b&&fail_success)throw 42;accepted=b;}void set_failure_code(unsigned n){code=n;}
 void set_client_nonce(const std::string&s){nonce=s;}void set_protocol_major(unsigned n){major=n;}
 void set_protocol_minor(unsigned n){minor=n;}void set_bridge_generation(std::uint64_t n){generation=n;}
 void set_df_version(const std::string&s){df=s;}void set_dfhack_version(const std::string&s){dfhack=s;}
 void set_observation(const std::string&s){observation=s;}void set_effect_record(const std::string&s){record=s;}
 void set_replayed(bool b){replayed=b;}
};
}
struct RPCService {
 std::vector<std::string> names;std::vector<int> flags;
 template<class F> void addFunction(const char*name,F,int flag){names.emplace_back(name);flags.push_back(flag);}
};
'''
DRIVER = r'''
#include <iostream>
#include <functional>
#include "bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp"
unsigned checks=0,groups=0;std::string group;
void check(bool v,const char *message){++checks;if(!v)throw std::runtime_error(message);}
std::string hex(const std::string&s){const char*h="0123456789abcdef";std::string out;for(unsigned char c:s){out+=h[c>>4];out+=h[c&15];}return out;}
struct Fixture {
 df::building holder;df::job job;df::unit unit;wire::Request r;wire::Reply p;color_ostream out;
 Fixture(){engine=js::Engine(7);TrackedBool::writes=0;TrackedBool::fail=false;Job::fail_read=false;Job::fail_after_write=false;
  Core::getInstance().loaded=true;Core::getInstance().map=true;Core::getInstance().vinfo=std::make_shared<VersionInfo>();VersionInfo::failure=0;Version::null_version=false;
  World::year=105;World::tick=3;World::paused=true;World::site=1;World::fortress=true;World::folder="region1";
  df::global::counter=10;df::global::job_next_id=&df::global::counter;Job::current=&job;job.holder=&holder;}
 void observe(){r.mask=2;ReadJob(out,&r,&p);check(p.accepted&&!p.observation.empty(),"read job");}
 void prepare(const std::string &key="job-001",bool desired=true){observe();r.witness=dfmcp_snapshot::sha256(p.observation);
  r.key=key;r.desired=desired;r.plan=js::plan_digest(r.id,desired,r.witness);r.mask=31;
  PrepareSuspension(out,&r,&p);check(p.accepted&&!p.record.empty(),"prepare job");
  const auto *record=engine.query(r.key,r.plan);check(record!=nullptr,"retained prepare");r.token=record->token;}
 const js::Record &commit(){r.mask=49;CommitSuspension(out,&r,&p);check(p.accepted,"commit RPC accepted");
  const auto *record=engine.query(r.key,r.plan);check(record!=nullptr,"commit retained");return *record;}
 void refused(){r.mask=49;CommitSuspension(out,&r,&p);check(p.accepted,"refusal is retained evidence");
  const auto*record=engine.query(r.key,r.plan);check(record&&record->state==js::State::Refused,"stale commit not refused");
  check(TrackedBool::writes==0,"refusal dispatched setter");check(record->receipt==record->proof(),"refusal receipt");}
};
int main(){
 try {
  setenv("DFMCP_JOB_CONTROL_TOKEN",std::string(32,'t').c_str(),1);
  auto run=[&](const char*name,auto test){group=name;++groups;test();};
  run("transaction",[]{Fixture f;f.prepare();check(TrackedBool::writes==0,"prepare writes");const auto token=f.r.token,plan=f.r.plan;
   f.r.mask=31;PrepareSuspension(f.out,&f.r,&f.p);check(f.p.accepted&&f.p.replayed,"exact prepare replay");
   check(engine.query(f.r.key,plan)->token==token,"prepare token changed");
   const auto &record=f.commit();check(record.state==js::State::Applied&&record.after_known&&record.after_suspended,"not applied");
   check(f.job.flags.bits.suspend&&World::paused&&f.job.flags.bits.repeat,"effect scope");check(TrackedBool::writes==1,"setter count");
   const auto encoded=record.encode();check(record.receipt==record.proof(),"receipt mismatch");
   f.commit();check(f.p.record==encoded&&TrackedBool::writes==1,"duplicate redispatched");
   f.r.mask=17;QuerySuspension(f.out,&f.r,&f.p);check(f.p.accepted&&f.p.record==encoded,"query receipt changed");
   f.r.key="absent";QuerySuspension(f.out,&f.r,&f.p);check(f.p.accepted&&f.p.record.empty(),"unknown key fabricated");
   f.prepare("resume",false);check(f.commit().state==js::State::Applied&&!f.job.flags.bits.suspend,"resume failed");
   check(TrackedBool::writes==2&&World::paused,"resume unpaused game");});
  run("eligibility",[]{for(int c=0;c<8;++c){Fixture f;
   switch(c){case 0:World::paused=false;break;case 1:f.job.worker=&f.unit;break;case 2:f.holder.stage=2;break;
    case 3:f.holder.kind=df::building_type::Other;break;case 4:f.job.holder=nullptr;break;case 5:f.job.supported=false;break;
    case 6:f.job.completion_timer=0;break;default:f.job.completion_timer=20;}
   f.observe();f.r.witness=dfmcp_snapshot::sha256(f.p.observation);f.r.key="blocked";f.r.plan=js::plan_digest(f.r.id,true,f.r.witness);f.r.mask=31;
   PrepareSuspension(f.out,&f.r,&f.p);check(!f.p.accepted&&f.p.code==4,"ineligible prepare accepted");check(engine.size()==0&&TrackedBool::writes==0,"ineligible side effect");}});
  run("changed-witness",[]{for(int c=0;c<16;++c){Fixture f;f.prepare();
   switch(c){case 0:World::tick++;break;case 1:World::paused=false;break;case 2:df::global::counter++;break;
    case 3:f.job.job_type++;break;case 4:f.job.pos.x++;break;case 5:f.job.flags.bits.repeat=false;break;
    case 6:f.job.flags.bits.suspend.value=true;break;case 7:f.job.reaction_name="different";break;case 8:f.holder.id++;break;
    case 9:f.job.worker=&f.unit;break;case 10:World::folder="region2";break;case 11:World::site++;break;
    case 12:f.job.items.push_back(&f.unit);break;case 13:f.job.job_items.elements.push_back(&f.unit);break;
    case 14:Job::current=nullptr;break;default:Job::fail_read=true;}
   f.refused();Job::fail_read=false;const auto first=engine.query(f.r.key,f.r.plan)->encode();f.r.mask=49;
   CommitSuspension(f.out,&f.r,&f.p);check(f.p.accepted&&f.p.record==first&&TrackedBool::writes==0,"refused key revived");}});
  run("competing-preparations",[]{Fixture f;f.prepare("a");const auto a=f.r;f.prepare("b");const auto b=f.r;
   f.r=a;check(f.commit().state==js::State::Applied,"first commit");f.r=b;
   check(f.commit().state==js::State::Refused&&TrackedBool::writes==1,"competing old prepare applied");
   Fixture noop;noop.prepare("a",false);const auto old=noop.r;noop.prepare("b",true);const auto later=noop.r;
   noop.r=old;check(noop.commit().state==js::State::Applied,"no-op commit");noop.r=later;
   check(noop.commit().state==js::State::Refused&&TrackedBool::writes==1,"no-op did not fence");});
  run("unknown-outcomes",[]{for(int c=0;c<2;++c){Fixture f;f.prepare();if(c==0)TrackedBool::fail=true;else Job::fail_after_write=true;
   check(f.commit().state==js::State::Unknown,"ambiguous effect claimed terminal");check(TrackedBool::writes==1,"ambiguous setter count");
   const auto old=engine.query(f.r.key,f.r.plan)->encode();TrackedBool::fail=false;Job::fail_after_write=false;
   f.commit();check(TrackedBool::writes==1&&f.p.record==old,"unknown redispatched");
   check(engine.query(f.r.key,f.r.plan)->receipt==std::string(32,'\0'),"unknown receipt invented");}
   Fixture lost;lost.prepare();wire::Reply::fail_success=true;lost.r.mask=49;CommitSuspension(lost.out,&lost.r,&lost.p);
   check(!lost.p.accepted&&TrackedBool::writes==1,"lost response effect");wire::Reply::fail_success=false;
   check(lost.commit().state==js::State::Applied&&TrackedBool::writes==1,"lost acknowledgement redispatched");});
  run("identity-and-shapes",[]{Fixture f;f.prepare();const auto original=f.r;
   for(int c=0;c<4;++c){f.r=original;if(c==0)f.r.plan[0]^=1;else if(c==1)f.r.token[0]^=1;else if(c==2)f.r.key="another";else f.r.token.pop_back();
    f.r.mask=49;CommitSuspension(f.out,&f.r,&f.p);check(!f.p.accepted&&f.p.code==7&&TrackedBool::writes==0,"commit identity bypass");}
   for(unsigned mask=0;mask<64;++mask){if(mask==49)continue;f.r=original;f.r.mask=mask;CommitSuspension(f.out,&f.r,&f.p);
    check(!f.p.accepted&&TrackedBool::writes==0,"commit argument shape accepted");}
   for(const std::string &key:std::vector<std::string>{"","with space","slash/path",std::string(129,'x')}){f.r=original;f.r.key=key;f.r.mask=31;PrepareSuspension(f.out,&f.r,&f.p);
    check(!f.p.accepted&&engine.size()==1,"invalid key retained");}
   f.r=original;f.r.desired=false;f.r.plan=js::plan_digest(f.r.id,false,f.r.witness);f.r.mask=31;PrepareSuspension(f.out,&f.r,&f.p);
   check(!f.p.accepted&&f.p.code==7&&engine.size()==1,"changed key content adopted");});
  run("lifecycle",[]{for(auto event:{SC_MAP_LOADED,SC_MAP_UNLOADED,SC_WORLD_LOADED,SC_WORLD_UNLOADED}){Fixture f;f.prepare();
   plugin_onstatechange(f.out,event);check(engine.generation()==8&&engine.size()==0,"incarnation not reset");f.r.mask=49;
   CommitSuspension(f.out,&f.r,&f.p);check(!f.p.accepted&&TrackedBool::writes==0,"old map effect dispatched");}
   for(auto event:{SC_PAUSED,SC_UNPAUSED}){Fixture f;f.prepare();plugin_onstatechange(f.out,event);f.refused();}
   Fixture f;f.prepare();plugin_onstatechange(f.out,SC_OTHER);check(f.commit().state==js::State::Applied,"unrelated event invalidated");
   engine=js::Engine(UINT64_MAX);f.r.mask=0;Handshake(f.out,&f.r,&f.p);check(!f.p.accepted,"saturated generation accepted");
   engine.reset();check(engine.generation()==UINT64_MAX,"generation wrapped");});
  run("auth-and-capture",[]{for(int c=0;c<12;++c){Fixture f;
   switch(c){case 0:f.r.auth[0]='x';break;case 1:f.r.auth=std::string(31,'t');break;case 2:f.r.auth=std::string(257,'t');break;
    case 3:f.r.nonce=std::string(15,'n');break;case 4:f.r.nonce=std::string(65,'n');break;case 5:f.r.minor=7;break;
    case 6:f.r.major=2;break;case 7:Version::null_version=true;break;case 8:VersionInfo::failure=1;break;
    case 9:VersionInfo::failure=2;break;case 10:Core::getInstance().vinfo.reset();break;default:f.r.mask=2;}
   Handshake(f.out,&f.r,&f.p);check(!f.p.accepted&&f.p.observation.empty()&&f.p.record.empty(),"invalid auth reply");
   check(engine.size()==0&&TrackedBool::writes==0,"auth side effect");}
   for(int c=0;c<9;++c){Fixture f;f.r.mask=2;
    switch(c){case 0:Core::getInstance().map=false;break;case 1:World::fortress=false;break;case 2:World::year=-1;break;
     case 3:World::tick=403200;break;case 4:df::global::job_next_id=nullptr;break;case 5:df::global::counter=3;break;
     case 6:f.job.general_refs.push_back(nullptr);break;case 7:World::folder=std::string("x\0y",3);break;default:f.job.completion_timer=-2;}
    ReadJob(f.out,&f.r,&f.p);check(!f.p.accepted&&f.p.observation.empty(),"malformed capture returned");}
   Fixture f;auto*s=plugin_rpcconnect(f.out);check(s->names==std::vector<std::string>({"Handshake","ReadJob","PrepareSuspension","CommitSuspension","QuerySuspension"}),"method surface");
   check(s->flags==std::vector<int>({0,0,0,0,0}),"unsuspended method");delete s;});
  run("deadlines-and-capacity",[]{Fixture f;const auto sample=read_job(3);auto read=[&](std::uint32_t){return sample;};
   auto time=js::Clock::time_point{}+std::chrono::seconds(100);
   for(int c=0;c<4;++c){js::Engine local(7);const auto observed=local.inspect(3,read);const auto witness=observed.witness();
    const auto plan=js::plan_digest(3,true,witness);const auto prepared=local.prepare("ttl",3,true,witness,plan,time,read).first;
    unsigned writes=0;const auto now=c==0?time-std::chrono::seconds(1):c==1?time+std::chrono::seconds(60):c==2?time+std::chrono::seconds(61):time;
    const auto&r=local.commit("ttl",plan,prepared->token,now,read,[&](auto,bool){++writes;});
    check(writes==(c==3?1u:0u),"TTL boundary");check(r.state==(c==3?js::State::NotApplied:js::State::Refused),"TTL status");}
   js::Engine local(7);const auto witness=local.inspect(3,read).witness();const auto plan=js::plan_digest(3,true,witness);
   for(unsigned i=0;i<js::MAX_RECORDS;++i)local.prepare("k"+std::to_string(i),3,true,witness,plan,time,read);
   bool refused=false;try{local.prepare("full",3,true,witness,plan,time,read);}catch(const js::Failure&){refused=true;}
   check(refused&&local.size()==js::MAX_RECORDS,"record capacity");check(local.prepare("k0",3,true,witness,plan,time,read).second,"full cache replay");});
  Fixture vector;vector.observe();const auto observation=vector.p.observation;vector.prepare();const auto &record=vector.commit();
  std::cout<<"{\"checks\":"<<checks<<",\"groups\":"<<groups<<",\"observation_hex\":\""<<hex(observation)
   <<"\",\"record_hex\":\""<<hex(record.encode())<<"\"}\n";
 }catch(const std::exception&e){std::cerr<<"FAIL "<<group<<": "<<e.what()<<"\n";return 1;}
}
'''

def verify_vectors(observation: bytes, record: bytes) -> dict:
    assert observation[:8] == b"DFMJS019" and record[:8] == b"DFMJSE19"
    generation, sequence, tick = struct.unpack_from(">QQQ", record, 8)
    job = struct.unpack_from(">I", record, 32)[0]
    desired = record[36]
    witness, plan, token = record[37:69], record[69:101], record[101:117]
    state, known, suspended = record[117:120]
    after_tick = struct.unpack_from(">Q", record, 120)[0]
    after_witness, receipt = record[128:160], record[160:192]
    key_length = struct.unpack_from(">H", record, 192)[0]
    key = record[194:]
    assert len(key) == key_length and key == b"job-001"
    assert (generation, sequence, tick, job, desired, state, known, suspended, after_tick) == (7, 0, 42336003, 3, 1, 2, 1, 1, 42336003)
    assert witness == hashlib.sha256(observation).digest()
    assert plan == hashlib.sha256(b"dfmcp-job-suspension-plan/1\0" + struct.pack(">IB", job, desired) + witness).digest()
    assert token == hashlib.sha256(b"dfmcp-job-suspension-token/1\0" + struct.pack(">QH", generation, len(key)) + key + plan).digest()[:16]
    assert receipt == hashlib.sha256(b"dfmcp-job-suspension-receipt/1\0" + struct.pack(">QH", generation, len(key)) + key + plan + token + bytes([state, known, suspended]) + struct.pack(">Q", after_tick) + after_witness).digest()
    after = bytearray(observation)
    struct.pack_into(">Q", after, 16, 1)
    after[84] |= 1
    assert after_witness == hashlib.sha256(after).digest()
    return {"witness": witness.hex(), "plan": plan.hex(), "token": token.hex(), "receipt": receipt.hex()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--ubsan", action="store_true")
    parser.add_argument("--fixture-dir", type=Path)
    parser.add_argument("--mutant", choices=["witness", "idempotency", "eligibility"])
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = root / "bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp"
    engine = root / "bridge/common/job_suspension.h"
    cache = root / "bridge/common/retained_snapshot.h"
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O1"]
    if args.ubsan:
        flags += ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
    with tempfile.TemporaryDirectory(prefix="dfmcp-job-control-") as directory:
        path = Path(directory)
        for original in (source, engine, cache):
            copied = path / original.relative_to(root)
            copied.parent.mkdir(parents=True, exist_ok=True)
            copied.write_bytes(original.read_bytes())
        if args.mutant:
            target = path / engine.relative_to(root)
            text = target.read_text()
            old, new = {
                "witness": ("current.eligible() && current.witness() == r.witness", "current.eligible()"),
                "idempotency": ("if (r.state != State::Prepared) return r;", "/* removed repeat-dispatch fence */"),
                "eligibility": ("require(before.eligible(), 4);", "/* removed eligibility preflight */"),
            }[args.mutant]
            assert text.count(old) == 1
            target.write_text(text.replace(old, new))
        (path / "mock.hpp").write_text(MOCK)
        names = ["Core.h", "Export.h", "MiscUtils.h", "PluginManager.h", "RemoteServer.h", "VersionInfo.h",
                 "modules/Job.h", "modules/World.h", "df/building.h", "df/building_type.h", "df/global_objects.h",
                 "df/job.h", "df/unit.h", "DfmcpJobControlV1_9.pb.h"]
        for name in names:
            header = path / name
            header.parent.mkdir(parents=True, exist_ok=True)
            header.write_text('#include "mock.hpp"\n')
        (path / "test.cpp").write_text(DRIVER)
        subprocess.run([args.compiler, *flags, "-I", str(path), str(path / "test.cpp"), "-o", str(path / "test")], check=True, timeout=60)
        run = subprocess.run([str(path / "test")], capture_output=True, text=True, timeout=30)
        if args.mutant:
            if run.returncode == 0:
                raise AssertionError(f"mutant {args.mutant} survived")
            print(json.dumps({"mutant": args.mutant, "rejected": True, "failure": run.stderr.strip()}))
            return
        if run.returncode:
            raise RuntimeError(run.stderr.strip() or f"native exit {run.returncode}")
        result = json.loads(run.stdout)
    observation = bytes.fromhex(result.pop("observation_hex"))
    record = bytes.fromhex(result.pop("record_hex"))
    result["vectors"] = verify_vectors(observation, record)
    if args.fixture_dir:
        args.fixture_dir.mkdir(parents=True, exist_ok=True)
        (args.fixture_dir / "job_suspension_observation_v1_9.hex").write_text(observation.hex() + "\n")
        (args.fixture_dir / "job_suspension_effect_v1_9.hex").write_text(record.hex() + "\n")
    result.update(compiler=args.compiler, flags=flags,
                  source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                  engine_sha256=hashlib.sha256(engine.read_bytes()).hexdigest(),
                  evidence="Actual plugin and engine with explicit DFHack/protobuf doubles; not a real plugin build, Rust, live game or admission")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
