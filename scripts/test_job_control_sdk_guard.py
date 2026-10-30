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
#include <iostream>
#include "bridge/dfhack-job-control-v1_9/dfmcp_job_control_v1_9.cpp"
int main() {
  setenv("DFMCP_JOB_CONTROL_TOKEN",std::string(32,'t').c_str(),1);
  df::world world; df::global::world=&world;
  df::building holder; df::job a,b; a.id=3; b.id=4; a.holder=&holder;
  df::job_list_link first, second; first.item=&a; first.next=&second; second.item=&b;
  world.jobs.list.next=&first;
  wire::Request request; wire::Reply reply; color_ostream out; request.mask=2;
  ReadJob(out,&request,&reply);
  if (!reply.accepted || reply.observation.empty()) return 1;
  auto observation=engine.inspect(3,read_job);
  if (observation.supported || observation.eligible()) return 2;
  request.witness=observation.witness(); request.key="job-001";
  request.desired=true; request.plan=js::plan_digest(3,true,request.witness); request.mask=31;
  PrepareSuspension(out,&request,&reply);
  if (reply.accepted || reply.code!=4 || engine.size()!=0 || TrackedBool::writes) return 3;
  try { set_suspended(3,true); return 4; } catch (const js::Failure &) {}
  if (TrackedBool::writes) return 5;
  // Even a match at the first link cannot hide a duplicate, null, or cycle later.
  b.id=3;
  try { (void)read_job(3); return 6; } catch (const js::Failure &) {}
  b.id=4; second.item=nullptr;
  try { (void)read_job(3); return 7; } catch (const js::Failure &) {}
  second.item=&b; second.next=&first;
  try { (void)read_job(3); return 8; } catch (const js::Failure &) {}
  second.next=nullptr; request.mask=2; request.id=9; ReadJob(out,&request,&reply);
  if (reply.accepted || reply.code!=4 || !reply.observation.empty()) return 9;
  std::vector<df::job> many_jobs(65537);
  std::vector<df::job_list_link> many_links(65537);
  for (std::size_t i=0;i<many_jobs.size();++i) {
    many_jobs[i].id=static_cast<int>(i); many_links[i].item=&many_jobs[i];
    many_links[i].next=i+1<many_links.size()?&many_links[i+1]:nullptr;
  }
  world.jobs.list.next=&many_links[0];
  try { (void)find_job(0); return 10; } catch (const js::Failure &) {}
  df::global::world=nullptr;
  try { (void)find_job(3); return 11; } catch (const js::Failure &) {}
  std::cout << "{\"checks\":11,\"status\":\"passed\"}\n";
}
'''


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    mock = MOCK.replace(
        "namespace global {inline int counter=10;inline int *job_next_id=&counter;}",
        "struct job_list_link { job *item=nullptr; job_list_link *next=nullptr; };\n"
        "struct world { struct { job_list_link list; } jobs; };\n"
        "namespace global {inline int counter=10;inline int *job_next_id=&counter;"
        "inline df::world *world=nullptr;}",
    )
    # Deliberately omit the two functions that the real SDK does not expose.
    mock = mock.replace(
        " inline df::job *getJob(int id){if(fail_read||(fail_after_write&&TrackedBool::writes))throw 42;return current&&current->id==id?current:nullptr;}\n", ""
    ).replace(" inline bool isSupportedJob(df::job*j){return j->supported;}\n", "")
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
                     "df/world.h", "df/job_list_link.h", "DfmcpJobControlV1_9.pb.h"]:
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
