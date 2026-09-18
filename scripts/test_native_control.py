#!/usr/bin/env python3
"""Compile and execute production control handlers with explicit boundary doubles.

No DFHack process or generated protobuf is exercised. No network is used.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path("bridge/dfhack-control-v1_7/dfmcp_control_v1_7.cpp")
CRYPTO = Path("bridge/common/retained_snapshot.h")
DRIVER = Path("tests/native/control/pause_dispatch_test.cpp")
STUB = Path("tests/native/control/dfhack_stubs.h")


def run(compiler, source, build, sanitize):
    headers = build / "headers"
    (headers / "modules").mkdir(parents=True)
    for name in ["Core.h", "Export.h", "PluginManager.h", "RemoteServer.h", "VersionInfo.h",
                 "modules/World.h", "DfmcpControlV1_7.pb.h"]:
        (headers / name).write_text('#include "dfhack_stubs.h"\n')
    executable = build / "control-tests"
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic"]
    flags += ["-O1", "-g", "-fsanitize=undefined", "-fno-sanitize-recover=all"] if sanitize else ["-O2"]
    subprocess.run([compiler, *flags, "-I", str(headers), "-I", str(ROOT / STUB.parent),
                    '-DDFMCP_CONTROL_SOURCE="' + str(source) + '"', str(ROOT / DRIVER),
                    "-o", str(executable)], check=True, timeout=60, capture_output=True, text=True)
    environment = dict(os.environ, DFMCP_CONTROL_TOKEN="t" * 32)
    return subprocess.run([str(executable)], env=environment, timeout=15,
                          capture_output=True, text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="c++")
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--mutation-check", action="store_true")
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if not compiler:
        parser.error("C++ compiler unavailable")
    # Digest exactly the production and test bytes used by this run.
    inputs = {str(p): hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
              for p in [SOURCE, CRYPTO, DRIVER, STUB, Path(__file__).relative_to(ROOT)]}
    with tempfile.TemporaryDirectory(prefix="dfmcp-control-tests-") as tmp:
        build = Path(tmp)
        result = run(compiler, ROOT / SOURCE, build / "positive", args.sanitize)
        if result.returncode:
            raise RuntimeError(result.stderr or result.stdout)
        report = json.loads(result.stdout)
        report.update(compiler=compiler, undefined_behavior_sanitizer=args.sanitize, input_sha256=inputs)
        if args.mutation_check:
            mutant = build / "mutant-source"
            for p in [SOURCE, CRYPTO]:
                (mutant / p).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / p, mutant / p)
            text = (mutant / SOURCE).read_text()
            guard = "if(!dispatch_fence.claim(r.guard,now,World::ReadPauseState(),dfmcp_pause::DispatchFence::Clock::now()))"
            if text.count(guard) != 1:
                raise RuntimeError("mutation no longer identifies exactly one dispatch gate")
            (mutant / SOURCE).write_text(text.replace(guard, "if(false)"))
            bad = run(compiler, mutant / SOURCE, build / "negative", args.sanitize)
            if bad.returncode != 1 or "stale_preparations:" not in bad.stderr:
                raise RuntimeError("removed dispatch gate did not fail the expected stale-preparation regression")
            report["removed_fence_mutant_rejected"] = True
        print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.stderr or str(error)) from error
