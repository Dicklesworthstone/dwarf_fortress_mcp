#!/usr/bin/env python3
"""Exercise actual pause outcome/replay code using explicitly injected DFHack mocks.

This is executable native-source regression evidence, not a generated-protobuf
build, a DFHack ABI check, a live game campaign, or production qualification.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

from test_live_control_native_mock import MOCK, expected_receipt, expected_token

DRIVER = r'''
#include <iomanip>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include "PRODUCER"
int checks=0;
void check(bool c){++checks;if(!c)throw std::runtime_error("check "+std::to_string(checks));}
std::string hex(const std::string&v){std::ostringstream s;for(unsigned char c:v)s<<std::hex<<std::setw(2)<<std::setfill('0')<<int(c);return s.str();}
wire::Request prepare(color_ostream &out,const std::string &key) {
    wire::Request r;wire::Reply reply;
    r.key=key;r.digest=std::string(32,'d');r.paused_present=true;r.paused_value=true;r.tick_present=true;
    r.tick=uint64_t(World::year)*403200ull+World::tick;
    PreparePause(out,&r,&reply);check(reply.accepted);check(!reply.known);check(reply.prepare.size()==16);
    r.prepare=reply.prepare;return r;
}
int main(){
    setenv("DFMCP_CONTROL_TOKEN",std::string(32,'t').c_str(),1);
    color_ostream out;wire::Reply reply;
    auto r=prepare(out,"moving-clock");const auto original_tick=r.tick;const auto token=r.prepare;
    QueryPause(out,&r,&reply);
    check(reply.accepted&&reply.known&&!reply.applied);check(!reply.paused);check(reply.receipt.empty());check(World::set_calls==0);
    ++World::tick;
    PreparePause(out,&r,&reply);check(reply.accepted);check(reply.prepare==token);check(reply.observed==original_tick);
    auto other=r;other.tick++;PreparePause(out,&other,&reply);check(!reply.accepted&&reply.code==7);
    other=r;other.paused_value=false;PreparePause(out,&other,&reply);check(!reply.accepted&&reply.code==7);
    other=r;other.digest=std::string(32,'x');PreparePause(out,&other,&reply);check(!reply.accepted&&reply.code==7);
    CommitPause(out,&r,&reply);check(reply.accepted&&reply.applied&&reply.paused);check(World::set_calls==1);
    const auto applied_receipt=reply.receipt;const auto applied_tick=reply.observed;
    check(applied_tick==original_tick+1);check(applied_receipt.size()==32);
    ++World::tick;World::paused=false; // External change cannot rewrite historical evidence.
    PreparePause(out,&r,&reply);check(reply.accepted);check(reply.prepare==token);check(reply.receipt==applied_receipt);
    CommitPause(out,&r,&reply);check(reply.receipt==applied_receipt);check(reply.observed==applied_tick);check(World::set_calls==1);
    QueryPause(out,&r,&reply);check(reply.receipt==applied_receipt);check(reply.paused);check(!World::paused);

    // Setter no-op: actual opposite state is reported, never the requested target.
    r=prepare(out,"setter-noop");World::ignore_set=true;
    CommitPause(out,&r,&reply);check(reply.accepted&&reply.known&&!reply.applied);check(!reply.paused);check(reply.code==5);
    check(reply.receipt.size()==32);const auto failed_receipt=reply.receipt;const auto failed_tick=reply.observed;
    const auto calls=World::set_calls;World::ignore_set=false;
    CommitPause(out,&r,&reply);check(World::set_calls==calls);check(reply.receipt==failed_receipt);
    PreparePause(out,&r,&reply);check(reply.accepted);check(reply.receipt==failed_receipt);check(!reply.paused);

    // Invalid or regressed clock refuses before the setter.
    r=prepare(out,"bad-before");const auto before=World::tick;const auto before_calls=World::set_calls;
    World::tick=403200;CommitPause(out,&r,&reply);check(!reply.accepted&&reply.code==5);check(World::set_calls==before_calls);
    World::tick=before-1;CommitPause(out,&r,&reply);check(!reply.accepted&&reply.code==6);check(World::set_calls==before_calls);
    World::tick=before;QueryPause(out,&r,&reply);check(reply.receipt.empty());check(!reply.applied);

    // Observation failure AFTER a setter remains receipt-less, including duplicate commits.
    r=prepare(out,"bad-after");World::invalidate_clock=true;
    CommitPause(out,&r,&reply);check(reply.accepted&&reply.known&&!reply.applied);check(reply.receipt.empty());check(World::paused);
    const auto after_calls=World::set_calls;World::invalidate_clock=false;World::tick=before;
    CommitPause(out,&r,&reply);check(reply.receipt.empty());check(World::set_calls==after_calls);
    QueryPause(out,&r,&reply);check(reply.receipt.empty());check(!reply.applied);

    // An exception after setting the target cannot authorize a second setter call.
    World::paused=false;r=prepare(out,"throw-after");World::throw_after=true;
    bool threw=false;try{CommitPause(out,&r,&reply);}catch(const std::runtime_error&){threw=true;}
    check(threw);check(World::paused);const auto throw_calls=World::set_calls;World::throw_after=false;
    QueryPause(out,&r,&reply);check(reply.accepted&&reply.known);check(reply.receipt.empty());check(!reply.applied);
    CommitPause(out,&r,&reply);check(reply.receipt.empty());check(World::set_calls==throw_calls);

    // Auth failures and epoch reset retain their original boundaries.
    const auto old_generation=generation;plugin_onstatechange(out,SC_WORLD_UNLOADED);
    check(generation==old_generation+1);QueryPause(out,&r,&reply);check(reply.accepted&&!reply.known);
    CommitPause(out,&r,&reply);check(!reply.accepted&&reply.code==7);check(World::set_calls==throw_calls);
    r.token="short";Handshake(out,&r,&reply);check(!reply.accepted);check(World::set_calls==throw_calls);
    auto *service=plugin_rpcconnect(out);
    check(service->names==std::vector<std::string>({"Handshake","PreparePause","CommitPause","QueryPause"}));
    check(service->flags==std::vector<int>({0,0,0,0}));delete service;
    std::cout<<"{\"checks\":"<<checks<<",\"generation\":"<<old_generation<<",\"prepare_tick\":"<<original_tick
        <<",\"applied_tick\":"<<applied_tick<<",\"failed_tick\":"<<failed_tick
        <<",\"token\":\""<<hex(token)<<"\",\"applied_receipt\":\""<<hex(applied_receipt)
        <<"\",\"failed_receipt\":\""<<hex(failed_receipt)<<"\"}\n";
}
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--output')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    producer = root / 'bridge/dfhack-control-v1_7/dfmcp_control_v1_7.cpp'
    setter = 'inline void SetPauseState(bool v){++set_calls;paused=v;}'
    if MOCK.count(setter) != 1:
        raise RuntimeError('shared mock setter changed; review fault injection')
    mock = MOCK.replace(setter, '''inline bool ignore_set=false,invalidate_clock=false,throw_after=false;
 inline void SetPauseState(bool v){++set_calls;if(!ignore_set)paused=v;
 if(invalidate_clock) {tick=403200;}
 if(throw_after)throw std::runtime_error("injected after setter");}''')
    mock = '#include <stdexcept>\n' + mock
    with tempfile.TemporaryDirectory(prefix='dfmcp-control-outcomes-') as directory:
        directory = pathlib.Path(directory)
        (directory / 'mock.hpp').write_text(mock)
        for name in ['Core.h', 'Export.h', 'PluginManager.h', 'RemoteServer.h',
                     'VersionInfo.h', 'modules/World.h', 'DfmcpControlV1_7.pb.h']:
            path = directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('#include "mock.hpp"\n')
        (directory / 'test.cpp').write_text(DRIVER.replace('PRODUCER', str(producer)))
        flags = ['-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic']
        subprocess.run([args.compiler, *flags, '-I', str(directory),
                        str(directory / 'test.cpp'), '-o', str(directory / 'test')],
                       check=True, timeout=60)
        result = json.loads(subprocess.run([str(directory / 'test')], capture_output=True,
                                          text=True, check=True, timeout=15).stdout)
    digest = b'd' * 32
    assert result['token'] == expected_token(result['generation'], 'moving-clock', digest,
                                             result['prepare_tick'], True).hex()
    assert result['applied_receipt'] == expected_receipt(result['generation'], 'moving-clock',
                                                        digest, True, result['applied_tick']).hex()
    assert result['failed_receipt'] == expected_receipt(result['generation'], 'setter-noop',
                                                       digest, True, result['failed_tick']).hex()
    result.update(compiler=args.compiler, flags=flags,
                  source_sha256=hashlib.sha256(producer.read_bytes()).hexdigest(),
                  evidence='mock_interfaces_not_real_dfhack_or_rust_execution')
    encoded = json.dumps(result, indent=2) + '\n'
    if args.output:
        pathlib.Path(args.output).write_text(encoded)
    print(encoded, end='')


if __name__ == '__main__':
    main()
