#!/usr/bin/env python3
"""Execute the actual citizen codec with explicit DFHack/serialization doubles.

This is not a real DFHack plugin build, generated-protobuf test, Rust test, or
live qualification. --source can point at an older header for regression probes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

MOCK = r'''
#pragma once
#include <algorithm>
#include <array>
#include <cstdint>
#include <string>
#include <vector>
namespace df {
struct coord { int32_t x=1,y=2,z=3; };
enum class job_skill { MINING=0, LAST=256 };
struct unit {
 int32_t id=42,profession=3,stress=3; coord pos;
 std::string name="Urist",race="dwarf";
 bool citizen=true,resident=false,visible_name=true,available=true;
 std::array<std::array<int,3>,257> skills{};
};
}
#define ENUM_LAST_ITEM(kind) df::kind::LAST
#define ENUM_KEY_STR(kind,value) ("skill_"+std::to_string(static_cast<int>(value)))
inline bool is_valid_enum_item(df::job_skill skill) { return static_cast<int>(skill)>=0&&static_cast<int>(skill)<=256; }
inline std::size_t largest_conversion_input=0;
inline std::string DF2UTF(const std::string &value) {
 largest_conversion_input=std::max(largest_conversion_input,value.size());
 static const char *high[]={CP437_HIGH};
 std::string out;
 for(unsigned char c:value) { if(c<128)out.push_back(static_cast<char>(c));else out+=high[c-128]; }
 return out;
}
namespace DFHack {
namespace Units {
inline std::vector<df::unit*> citizens; inline bool roster_available=true;
inline bool getCitizens(std::vector<df::unit*>&out,bool,bool) { out=citizens;return roster_available; }
inline bool isCitizen(df::unit*u,bool) { return u->citizen; }
inline bool isResident(df::unit*u,bool) { return u->resident; }
inline const std::string* getVisibleName(df::unit*u) { return u->visible_name?&u->name:nullptr; }
inline std::string getRaceReadableName(df::unit*u) { return u->race; }
inline int getProfession(df::unit*u) { return u->profession; }
inline int getStressCategory(df::unit*u) { return u->stress; }
inline df::coord getPosition(df::unit*u) { return u->pos; }
inline bool isJobAvailable(df::unit*u,bool) { return u->available; }
inline bool isAlive(df::unit*) { return true; }
inline bool isSane(df::unit*) { return true; }
inline bool isActive(df::unit*) { return true; }
inline bool isVisible(df::unit*) { return true; }
inline bool isBaby(df::unit*) { return false; }
inline bool isChild(df::unit*) { return false; }
inline bool isAdult(df::unit*) { return true; }
inline int getNominalSkill(df::unit*u,df::job_skill s,bool) { return u->skills.at(static_cast<size_t>(s))[0]; }
inline int getEffectiveSkill(df::unit*u,df::job_skill s) { return u->skills.at(static_cast<size_t>(s))[1]; }
inline int getExperience(df::unit*u,df::job_skill s,bool) { return u->skills.at(static_cast<size_t>(s))[2]; }
}
namespace Translation {
inline std::string translateName(const std::string*name,bool) { return *name; }
}
}
namespace dfmcp_spatial_capture {
inline void u32(std::string &out,uint32_t n) { for(int shift=24;shift>=0;shift-=8)out.push_back(static_cast<char>(n>>shift)); }
inline void i32(std::string &out,int32_t n) { u32(out,static_cast<uint32_t>(n)); }
inline void u16(std::string &out,uint16_t n) { out.push_back(static_cast<char>(n>>8));out.push_back(static_cast<char>(n)); }
inline void text(std::string &out,const std::string &s) { u16(out,static_cast<uint16_t>(s.size()));out+=s; }
// UTF-8 structure is independently decoded by the Python side. This boundary
// double checks only codec length/NUL contracts, not the real spatial helper.
inline bool utf8(const std::string &s,size_t max,bool empty=false) { return s.size()<=max&&(empty||!s.empty())&&s.find('\0')==std::string::npos; }
}
'''

DRIVER = r'''
#include <iostream>
#include <limits>
#include <stdexcept>
#include "citizen_capture.h"
namespace codec=dfmcp_citizen_capture;
namespace units=DFHack::Units;
unsigned checks=0,groups=0;std::string current;
void check(bool ok,const char*message) { ++checks;if(!ok)throw std::runtime_error(message); }
uint32_t u32(const std::string&s,size_t p) { return (uint32_t(uint8_t(s.at(p)))<<24)|(uint32_t(uint8_t(s.at(p+1)))<<16)|(uint32_t(uint8_t(s.at(p+2)))<<8)|uint8_t(s.at(p+3)); }
std::string text(const std::string&s,size_t &p) { auto n=(uint32_t(uint8_t(s.at(p)))<<8)|uint8_t(s.at(p+1));p+=2;auto result=s.substr(p,n);p+=n;return result; }
int main(int argc,char**argv) {
 try {
  const std::string selected=argc>1?argv[1]:"";
  auto run=[&](const char*name,auto test) { if(!selected.empty()&&selected!=name)return;current=name;++groups;units::roster_available=true;units::citizens.clear();test(); };
  run("encoding",[&] {
   df::unit u;u.name="Urist "+std::string(1,char(0x82))+"lan";u.race="n"+std::string(1,char(0xa4))+"and";
   units::citizens={&u};std::string out;check(codec::capture(1,4096,out)==0,"capture failed");size_t p=16;
   check(text(out,p)=="Urist \xc3\xa9lan","CP437 name was truncated or misencoded");
   check(text(out,p)=="n\xc3\xb1" "and","CP437 race was truncated or misencoded");
   for(int c=128;c<256;++c) { u.name=std::string(1,char(c));check(codec::capture(1,4096,out)==0,"high-byte capture failed");p=16;check(text(out,p)==DF2UTF(u.name),"high-byte conversion mismatch"); }
  });
  run("boundaries",[&] {
   df::unit u;units::citizens={&u};std::string out;
   for(size_t n: {size_t(253),size_t(254),size_t(255),size_t(256),size_t(257)}) {
    u.name=std::string(n,'a')+char(0x82);check(codec::capture(1,4096,out)==0,"bounded capture failed");size_t p=16;
    auto name=text(out,p);check(name==std::string(std::min(n,size_t(256)),'a')+(n+2<=256?"\xc3\xa9":""),"split or dropped fitting scalar");
   }
   u.name=std::string(1000000,char(0x82));u.race=std::string(1000000,char(0xb0));largest_conversion_input=0;
   check(codec::capture(1,4096,out)==0,"large text failed");size_t p=16;check(text(out,p).size()==256,"name bound");check(text(out,p).size()==126,"race scalar bound");
   check(largest_conversion_input<=256,"unbounded conversion allocation");
   u.name=std::string("a\0b",3);check(codec::capture(1,4096,out)==5,"NUL accepted");
   u.visible_name=false;check(codec::capture(1,4096,out)==0,"absent visible name refused");p=16;check(text(out,p).empty(),"absent visible name invented");
  });
  run("roster",[&] {
   df::unit a,b;b.id=43;std::string out;
   units::citizens={&a,nullptr};check(codec::capture(2,4096,out)==5,"null roster member silently dropped");
   units::citizens={&a,&a};check(codec::capture(2,4096,out)==5,"duplicate accepted");
   units::citizens={&a};a.id=-1;check(codec::capture(2,4096,out)==5,"negative ID accepted");a.id=42;
   a.resident=true;check(codec::capture(2,4096,out)==5,"resident accepted");a.resident=false;
   a.citizen=false;check(codec::capture(2,4096,out)==5,"non-citizen accepted");a.citizen=true;
   units::roster_available=false;check(codec::capture(2,4096,out)==5,"failed roster accepted");units::roster_available=true;
   units::citizens={&b,&a};check(codec::capture(2,4096,out)==0,"valid roster rejected");const auto sorted=out;
   check(u32(out,12)==42,"roster not sorted");units::citizens={&a,&b};check(codec::capture(2,4096,out)==0&&out==sorted,"input-order dependent encoding");
   units::citizens.clear();check(codec::capture(1,4096,out)==0&&out.size()==12&&u32(out,8)==0,"empty roster contract");
  });
  run("skills",[&] {
   df::unit u;units::citizens={&u};std::string out;
   for(size_t column=0;column<3;++column) { u.skills={};u.skills[0][column]=-1;check(codec::capture(1,4096,out)==5,"negative-only skill silently omitted");
    u.skills[0][(column+1)%3]=1;check(codec::capture(1,4096,out)==5,"mixed negative skill serialized"); }
   u.skills={};u.skills[0]={0,0,1};check(codec::capture(1,4096,out)==0,"experience-only skill omitted");check(out.find("skill_0")!=std::string::npos,"experience-only skill lost");
   u.skills[0]={1,0,0};check(codec::capture(1,4096,out)==0,"rusted skill lost");
   u.skills[0]={std::numeric_limits<int>::max(),0,0};check(codec::capture(1,4096,out)==0,"valid signed maximum refused");
   for(auto &skill:u.skills) { skill={1,1,1}; }
   check(codec::capture(1,16384,out)==3,"257 skills accepted");
   u.skills.back()={};check(codec::capture(1,16384,out)==0,"256 skills refused");
   std::vector<df::unit> many(513,u);units::citizens.clear();for(size_t i=0;i<many.size();++i){many[i].id=static_cast<int>(i);units::citizens.push_back(&many[i]);}
   check(codec::capture(513,16*1024*1024,out)==3,"global skill ceiling bypassed");
   units::citizens.pop_back();check(codec::capture(512,16*1024*1024,out)==0,"global skill boundary refused");
  });
  run("bounds",[&] {
   df::unit u;units::citizens={&u};std::string out;
   for(auto max: {0u,4097u})check(codec::capture(max,4096,out)==3,"invalid roster bound accepted");
   for(auto bytes:{size_t(1023),size_t(16*1024*1024+1)})check(codec::capture(1,bytes,out)==3,"invalid byte bound accepted");
   units::citizens={&u,&u};check(codec::capture(1,4096,out)==3,"roster overflow");units::citizens={&u};
   u.stress=7;check(codec::capture(1,4096,out)==5,"stress overflow");u.stress=-1;check(codec::capture(1,4096,out)==5,"negative stress");u.stress=3;
   u.profession=-1;check(codec::capture(1,4096,out)==5,"negative profession");u.profession=3;
   for(size_t i=0;i<60;++i) { u.skills[i]={1,1,1}; }
   check(codec::capture(1,1024,out)==3,"payload overflow");
  });
  run("utf8",[&] {
   const std::string value="a\xc3\xa9\xe2\x96\x91\xf0\x9f\x8f\x94";
   check(codec::utf8_width(value,value.size())==0,"end offset accepted");
   check(codec::utf8_width(value,std::numeric_limits<size_t>::max())==0,"overflowing offset accepted");
   for(size_t n=0;n<=value.size();++n) { auto prefix=codec::bounded_utf8(value,n);check(prefix.size()<=n,"UTF-8 bound exceeded");
    check(prefix.size()==0||prefix.size()==1||prefix.size()==3||prefix.size()==6||prefix.size()==10,"split scalar"); }
   for(const std::string &invalid:{std::string("\xc0\x80"),std::string("\xed\xa0\x80"),std::string("\xf4\x90\x80\x80"),std::string("\xe2\x96")})
    check(codec::bounded_utf8(invalid,256).empty(),"invalid scalar accepted");
  });
  check(groups!=0,"unknown test group");
  df::unit u;u.name="Urist "+std::string(1,char(0x82))+"lan";u.skills[0]={7,6,123};units::citizens={&u};std::string out;
  check(codec::capture(1,4096,out)==0,"wire fixture capture");
  const char*digits="0123456789abcdef";std::string hex;for(unsigned char c:out){hex+=digits[c>>4];hex+=digits[c&15];}
  std::cout<<"{\"checks\":"<<checks<<",\"groups\":"<<groups<<",\"wire_hex\":\""<<hex<<"\"}\n";
 } catch(const std::exception &e) { std::cerr<<"FAIL "<<current<<": "<<e.what()<<"\n";return 1; }
}
'''


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--group", default="")
    parser.add_argument("--ubsan", action="store_true")
    parser.add_argument("--permit-misleading-indentation", action="store_true",
                        help="Only for executing regressions against older source")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = args.source or root / "bridge/common/citizen_capture.h"
    source_bytes = source.read_bytes()
    table = ",".join('"' + "".join(f"\\x{b:02x}" for b in bytes([c]).decode("cp437").encode()) + '"'
                     for c in range(128, 256))
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O1"]
    if args.ubsan:
        flags += ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
    if args.permit_misleading_indentation:
        flags += ["-Wno-error=misleading-indentation"]
    with tempfile.TemporaryDirectory(prefix="dfmcp-citizen-codec-") as directory:
        directory = Path(directory)
        (directory / "citizen_capture.h").write_bytes(source_bytes)
        (directory / "mock.hpp").write_text(MOCK.replace("CP437_HIGH", table))
        for name in ["spatial_capture.h", "MiscUtils.h", "modules/Translation.h", "modules/Units.h",
                     "df/coord.h", "df/job_skill.h", "df/unit.h"]:
            path = directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('#include "mock.hpp"\n')
        (directory / "test.cpp").write_text(DRIVER)
        executable = directory / "test"
        subprocess.run([args.compiler, *flags, "-I", str(directory), str(directory / "test.cpp"),
                        "-o", str(executable)], check=True, timeout=60)
        completed = subprocess.run([str(executable), args.group], capture_output=True,
                                   text=True, check=False, timeout=30)
        if completed.returncode:
            raise RuntimeError(completed.stderr.strip() or f"native exit {completed.returncode}")
        result = json.loads(completed.stdout)
    wire = bytes.fromhex(result.pop("wire_hex"))
    assert wire[:8] == b"DFMC1800" and int.from_bytes(wire[8:12], "big") == 1
    assert int.from_bytes(wire[12:16], "big") == 42
    length = int.from_bytes(wire[16:18], "big")
    assert wire[18:18+length].decode("utf-8") == "Urist élan"
    result.update(compiler=args.compiler, flags=flags, wire_sha256=hashlib.sha256(wire).hexdigest(),
                  source_sha256=hashlib.sha256(source_bytes).hexdigest(),
                  evidence="Actual citizen codec with explicit boundary doubles; not Rust, real DFHack/protobuf, live or repository qualification")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
