#!/usr/bin/env python3
"""Exercise actual job-control handler with bounded SDK-shaped list doubles.

This is not a real DFHack build, live-game test, or compatibility evidence.
"""
import json
from pathlib import Path
import re
import subprocess
import tempfile

from test_job_suspension_native import MOCK


DRIVER = r'''
#include <iostream>
#include "bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp"
unsigned checks=0;
void check(bool value,const char *message) {
  ++checks; if (!value) throw std::runtime_error(message);
}
template<class F> void refused(F operation) {
  bool failed=false;
  try { operation(); } catch (const js::Failure &) { failed=true; }
  check(failed,"malformed native state was accepted");
}
int main() {
 try {
  setenv("DFMCP_JOB_CONTROL_TOKEN",std::string(32,'t').c_str(),1);
  engine=js::Engine(7);
  df::world world; df::global::world=&world;
  df::building holder; df::job a,b; a.id=3; b.id=4; a.holder=&holder;
  int filter=0; a.job_items.elements.push_back(&filter); holder.jobs.push_back(&a);
  df::job_list_link first, second;
  first.item=&a; first.next=&second; first.prev=&world.jobs.list;
  second.item=&b; second.prev=&first; a.list_link=&first; b.list_link=&second;
  world.jobs.list.next=&first;
  wire::Request request; wire::Reply reply; color_ostream out; request.mask=2;
  ReadJob(out,&request,&reply);
  check(reply.accepted&&!reply.observation.empty(),"SDK-shaped positive read");
  auto observation=engine.inspect(3,read_job);
  check(observation.supported&&observation.eligible(),"closed policy rejected exact idle job");
  check(observation.type==69&&observation.type_key=="ConstructBed"&&observation.holder_type==13,
    "native enum fixture differs from the pinned SDK");
  request.witness=observation.witness(); request.key="job-001";
  request.desired=true; request.plan=js::plan_digest(3,true,request.witness); request.mask=31;
  PrepareSuspension(out,&request,&reply);
  check(reply.accepted&&engine.size()==1&&TrackedBool::writes==0,"nonmutating prepare");
  const auto *prepared=engine.query(request.key,request.plan);
  check(prepared!=nullptr,"original preparation missing"); request.token=prepared->token; request.mask=49;
  CommitSuspension(out,&request,&reply);
  const auto *applied=engine.query(request.key,request.plan);
  check(reply.accepted&&applied&&applied->state==js::State::Applied,"exact native setter not applied");
  check(TrackedBool::writes==1&&a.flags.bits.suspend&&a.flags.bits.repeat&&World::paused,
    "native setter changed more than the suspension bit");
  const auto receipt=reply.record; CommitSuspension(out,&request,&reply);
  check(reply.accepted&&reply.record==receipt&&TrackedBool::writes==1,"same-key redispatch");

  // The real supported field is a derived policy, never an operator-set flag.
  observation=engine.inspect(3,read_job); request.key="job-resume"; request.desired=false;
  request.witness=observation.witness(); request.plan=js::plan_digest(3,false,request.witness); request.mask=31;
  PrepareSuspension(out,&request,&reply); check(reply.accepted,"resume preparation refused");
  request.token=engine.query(request.key,request.plan)->token;
  a.posting_index=0; request.mask=49; CommitSuspension(out,&request,&reply);
  check(reply.accepted&&engine.query(request.key,request.plan)->state==js::State::Refused,
    "posting change after preparation did not refuse dispatch");
  check(TrackedBool::writes==1&&a.flags.bits.suspend,"posted job was modified");
  a.posting_index=-1; CommitSuspension(out,&request,&reply);
  check(reply.accepted&&TrackedBool::writes==1,"retired preparation was revived");

  a.job_type=df::job_type::CustomReaction; request.mask=2; ReadJob(out,&request,&reply);
  check(reply.accepted&&!engine.inspect(3,read_job).eligible(),"custom reaction inferred eligible");
  refused([&]{set_suspended(3,false);});
  check(TrackedBool::writes==1,"direct unsupported setter wrote a flag");
  a.job_type=static_cast<df::job_type>(32767); ReadJob(out,&request,&reply);
  const auto unknown=engine.inspect(3,read_job);
  check(reply.accepted&&unknown.type==32767&&unknown.type_key=="?32767?"&&!unknown.supported,
    "SDK unknown enum fallback was replaced or treated as supported");
  a.job_type=df::job_type::ConstructBed;

  // Even a match at the first link cannot hide a duplicate, null, or cycle later.
  b.id=3; refused([&]{(void)read_job(3);});
  b.id=4; second.item=nullptr; refused([&]{(void)read_job(3);});
  second.item=&b; second.next=&first; refused([&]{(void)read_job(3);});
  second.next=nullptr; second.prev=nullptr; refused([&]{(void)read_job(3);});
  second.prev=&first; b.list_link=&first; refused([&]{(void)read_job(3);});
  b.list_link=&second; b.id=10; refused([&]{(void)read_job(3);}); b.id=4;
  second.next=nullptr; request.mask=2; request.id=9; ReadJob(out,&request,&reply);
  check(!reply.accepted&&reply.code==4&&reply.observation.empty(),"missing selected job fabricated");
  std::vector<df::job> many_jobs(65537);
  std::vector<df::job_list_link> many_links(65537);
  df::global::counter=65538;
  for (std::size_t i=0;i<many_jobs.size();++i) {
    many_jobs[i].id=static_cast<int>(i); many_links[i].item=&many_jobs[i];
    many_jobs[i].list_link=&many_links[i];
    many_links[i].prev=i?&many_links[i-1]:&world.jobs.list;
    many_links[i].next=i+1<many_links.size()?&many_links[i+1]:nullptr;
  }
  world.jobs.list.next=&many_links[0];
  refused([&]{(void)find_job(0);});
  df::global::world=nullptr;
  refused([&]{(void)find_job(3);});
  check(TrackedBool::writes==1,"rejected state changed a native flag");
  std::cout << "{\"checks\":"<<checks<<",\"status\":\"passed\"}\n";
 } catch(const std::exception &error) {
  std::cerr<<error.what()<<"\n"; return 1;
 }
}
'''


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    # Do not silently strip invented APIs from a drifting fixture: reject them.
    # The handler must compile without either nonexistent public Job method.
    if re.search(r"\b(?:getJob|isSupportedJob)\s*\(", MOCK):
        raise AssertionError("the SDK double declares a fictional public Job API")
    with tempfile.TemporaryDirectory(prefix="dfmcp-job-sdk-guard-") as tmp:
        path = Path(tmp)
        for relative in ["bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp",
                         "bridge/common/job_suspension.h", "bridge/common/retained_snapshot.h"]:
            dest = path / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes((root / relative).read_bytes())
        (path / "mock.hpp").write_text(MOCK)
        for name in ["Core.h", "Export.h", "MiscUtils.h", "PluginManager.h", "RemoteServer.h",
                     "VersionInfo.h", "modules/Job.h", "modules/World.h", "df/building.h",
                     "df/building_type.h", "df/global_objects.h", "df/job.h", "df/unit.h",
                     "df/world.h", "df/job_list_link.h", "df/job_type.h", "DfmcpJobControlV1_9.pb.h"]:
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
