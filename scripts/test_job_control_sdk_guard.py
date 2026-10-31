#!/usr/bin/env python3
"""Exercise actual job-control handler with bounded SDK-shaped list doubles.

This is not a real DFHack build, live-game test, or compatibility evidence.
"""
import json
from pathlib import Path
import subprocess
import tempfile

from test_job_suspension_native import MOCK


DRIVER = r'''
#include <functional>
#include <iostream>
#include "bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp"
unsigned checks=0;
void check(bool v,const char *why) { ++checks; if(!v) throw std::runtime_error(why); }
struct Fixture {
 df::world world;df::building holder;df::job a,b;df::unit worker;
 df::general_ref holder_ref,worker_ref,dangling_ref,other_holder;
 df::job_list_link first,second;wire::Request r;wire::Reply reply;color_ostream out;
 Fixture() {
  engine=js::Engine(7);TrackedBool::writes=0;TrackedBool::fail=false;
  Job::fail_read=false;Job::fail_after_write=false;World::paused=true;
  df::global::counter=10;df::global::job_next_id=&df::global::counter;
  df::global::world=&world;a.id=3;b.id=4;holder.jobs={&a};
  holder_ref.building=&holder;worker_ref.type=df::general_ref_type::UNIT_WORKER;
  worker_ref.unit=&worker;worker_ref.id=worker.id;
  dangling_ref.type=df::general_ref_type::UNIT_WORKER;dangling_ref.id=100;
  other_holder.building=&holder;a.general_refs={&holder_ref};
  first.item=&a;first.next=&second;second.item=&b;world.jobs.list.next=&first;
 }
 void prepare() {
  r.mask=2;ReadJob(out,&r,&reply);check(reply.accepted,"read eligible job");
  const auto v=engine.inspect(3,read_job);check(v.supported&&v.eligible(),"positive eligibility unavailable");
  r.witness=v.witness();r.key="job-001";r.desired=true;
  r.plan=js::plan_digest(3,true,r.witness);r.mask=31;PrepareSuspension(out,&r,&reply);
  check(reply.accepted&&engine.size()==1&&TrackedBool::writes==0,"prepare changed job or failed");
  r.token=engine.query(r.key,r.plan)->token;
 }
 void refused() {
  const auto v=engine.inspect(3,read_job);check(!v.eligible(),"unsupported class eligible");
  r.witness=v.witness();r.key="blocked";r.plan=js::plan_digest(3,true,r.witness);r.mask=31;
  PrepareSuspension(out,&r,&reply);check(!reply.accepted&&reply.code==4&&engine.size()==0,"unsupported prepare retained");
  bool denied=false;try{set_suspended(3,true);}catch(const js::Failure &){denied=true;}
  check(denied&&TrackedBool::writes==0&&!a.flags.bits.suspend,"direct setter bypassed eligibility");
 }
};
int main() { try {
 setenv("DFMCP_JOB_CONTROL_TOKEN",std::string(32,'t').c_str(),1);
 for(auto type:{df::job_type::ConstructBed,df::job_type::ConstructDoor,
                df::job_type::ConstructThrone,df::job_type::ConstructTable}) {
  Fixture f;f.a.job_type=type;f.prepare();f.r.mask=49;CommitSuspension(f.out,&f.r,&f.reply);
  const auto *v=engine.query(f.r.key,f.r.plan);
  check(f.reply.accepted&&v&&v->state==js::State::Applied,"positive setter unavailable");
  check(f.a.flags.bits.suspend&&f.a.flags.bits.repeat&&World::paused&&TrackedBool::writes==1,"wrong effect scope");
  CommitSuspension(f.out,&f.r,&f.reply);check(f.reply.accepted&&TrackedBool::writes==1,"replay setter");
  set_suspended(3,false);check(!f.a.flags.bits.suspend&&TrackedBool::writes==2,"resume setter unavailable");
 }
 const std::vector<std::function<void(Fixture&)>> blocked={
  [](auto&f){f.a.job_type=df::job_type::ConstructBuilding;},
  [](auto&f){f.a.job_type=static_cast<df::job_type>(32767);},
  [](auto&f){f.holder.kind=df::building_type::Furnace;},
  [](auto&f){f.holder.kind=df::building_type::Other;},
  [](auto&f){f.holder.stage=2;},[](auto&f){f.a.general_refs.push_back(&f.worker_ref);},
  [](auto&f){f.a.completion_timer=0;},[](auto&){World::paused=false;},
  [](auto&f){f.a.flags.bits.working=true;},[](auto&f){f.a.flags.bits.fetching=true;},
  [](auto&f){f.a.flags.bits.special=true;},[](auto&f){f.a.flags.bits.bringing=true;},
  [](auto&f){f.a.flags.bits.item_lost=true;},[](auto&f){f.a.reaction_name="custom";},
  [](auto&f){f.a.specific_refs.push_back(&f.worker);},
  [](auto&f){f.holder.jobs.clear();},[](auto&f){f.holder.jobs={&f.b};},
  [](auto&f){f.holder.jobs={&f.a,&f.a};},[](auto&f){f.holder.jobs={&f.a,nullptr};},
  [](auto&f){f.b.id=f.a.id;f.holder.jobs={&f.a,&f.b};f.first.next=nullptr;},
  [](auto&f){f.b.id=-1;f.holder.jobs={&f.a,&f.b};f.first.next=nullptr;},
  [](auto&f){f.holder.jobs.resize(11,&f.a);},
  [](auto&f){f.a.general_refs={&f.dangling_ref,&f.worker_ref,&f.holder_ref};},
  [](auto&f){f.a.general_refs.push_back(&f.dangling_ref);},
  [](auto&f){f.a.general_refs.push_back(&f.other_holder);},
  [](auto&f){f.holder_ref.id=-1;},
  [](auto&f){f.holder_ref.type=df::general_ref_type::OTHER;},
  [](auto&f){f.a.general_refs.clear();}
 };
 for(const auto &mutate:blocked){Fixture f;mutate(f);f.refused();}
 // Each supported-job invalidator is checked again after preparation, before write.
 for(const auto &mutate:blocked){Fixture f;f.prepare();mutate(f);f.r.mask=49;
  CommitSuspension(f.out,&f.r,&f.reply);const auto *v=engine.query(f.r.key,f.r.plan);
  check(f.reply.accepted&&v&&v->state==js::State::Refused&&TrackedBool::writes==0,"changed eligibility dispatched");
 }
 {Fixture f;f.prepare();f.r.mask=49;f.r.auth[0]='x';CommitSuspension(f.out,&f.r,&f.reply);
  check(!f.reply.accepted&&TrackedBool::writes==0,"revoked credential dispatched");}
 for(unsigned c=0;c<4;++c){Fixture f;
  if(c==0)f.b.id=f.a.id;else if(c==1)f.second.item=nullptr;
  else if(c==2)f.second.next=&f.first;else f.r.id=9;
  f.r.mask=2;ReadJob(f.out,&f.r,&f.reply);
  check(!f.reply.accepted&&f.reply.observation.empty()&&TrackedBool::writes==0,"malformed list returned observation");
 }
 {Fixture f;std::vector<df::job> jobs(65537);std::vector<df::job_list_link> links(65537);
  for(std::size_t i=0;i<jobs.size();++i){jobs[i].id=static_cast<int>(i);links[i].item=&jobs[i];
   links[i].next=i+1<links.size()?&links[i+1]:nullptr;}
  f.world.jobs.list.next=&links[0];bool denied=false;try{(void)find_job(0);}catch(const js::Failure&){denied=true;}
  check(denied,"job-list bound bypassed");}
 {Fixture f;df::global::world=nullptr;bool denied=false;
  try{(void)find_job(3);}catch(const js::Failure&){denied=true;}
  check(denied,"absent world accepted");}
 std::cout<<"{\"checks\":"<<checks<<",\"positive_job_types\":4,\"negative_policies\":"<<blocked.size()<<",\"status\":\"passed\"}\n";
 }catch(const std::exception&e){std::cerr<<e.what()<<"\n";return 1;}
}
'''


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    mock = MOCK
    with tempfile.TemporaryDirectory(prefix="dfmcp-job-sdk-guard-") as tmp:
        path = Path(tmp)
        for relative in ["bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp",
                         "bridge/common/job_suspension.h", "bridge/common/retained_snapshot.h"]:
            dest = path / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes((root / relative).read_bytes())
        (path / "mock.hpp").write_text(mock)
        for name in ["Core.h", "Export.h", "MiscUtils.h", "PluginManager.h", "RemoteServer.h",
                     "VersionInfo.h", "modules/Job.h", "modules/World.h", "df/building.h",
                     "df/building_type.h", "df/global_objects.h", "df/job.h", "df/unit.h",
                     "df/world.h", "df/job_list_link.h", "df/job_type.h", "df/general_ref.h",
                     "df/general_ref_type.h", "DfmcpJobControlV1_9.pb.h"]:
            header = path / name
            header.parent.mkdir(parents=True, exist_ok=True)
            header.write_text('#include "mock.hpp"\n')
        (path / "test.cpp").write_text(DRIVER)
        subprocess.run(["c++", "-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic",
                        "-O1", "-I", str(path), str(path / "test.cpp"), "-o", str(path / "test")],
                       check=True, timeout=60)
        result = subprocess.run([str(path / "test")], capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise RuntimeError(f"guard test exited {result.returncode}: {result.stderr}")
        print(json.dumps({"result": json.loads(result.stdout),
                          "scope": "actual handler with SDK-shaped doubles; no native or live qualification"}))


if __name__ == "__main__":
    main()
