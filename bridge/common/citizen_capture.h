#pragma once
#include <algorithm>
#include <cstdint>
#include <limits>
#include <string>
#include <vector>
#include "MiscUtils.h"
#include "spatial_capture.h"
#include "modules/Translation.h"
#include "modules/Units.h"
#include "df/coord.h"
#include "df/job_skill.h"
#include "df/unit.h"

// Data-only strict-citizen codec used only inside one already-suspended native
// spatial RPC. It performs no RPC, cache publication, mutation, or background work.
namespace dfmcp_citizen_capture {
using namespace DFHack;
namespace base = dfmcp_spatial_capture;
constexpr std::size_t MAX_SKILLS_PER_CITIZEN=256;
constexpr std::size_t MAX_SKILLS_TOTAL=131072;
constexpr std::size_t MAX_SKILL_KEY_BYTES=96;

inline std::size_t utf8_width(const std::string &value,std::size_t offset){
    if(offset>=value.size())return 0;
    const auto lead=static_cast<unsigned char>(value[offset]);
    if(lead<=0x7f)return 1;
    if(lead>=0xc2&&lead<=0xdf){if(value.size()-offset<2)return 0;return (static_cast<unsigned char>(value[offset+1])&0xc0)==0x80?2:0;}
    if(lead>=0xe0&&lead<=0xef){
        if(value.size()-offset<3)return 0;
        const auto b=static_cast<unsigned char>(value[offset+1]);
        if((b&0xc0)!=0x80||(static_cast<unsigned char>(value[offset+2])&0xc0)!=0x80)return 0;
        if((lead==0xe0&&b<0xa0)||(lead==0xed&&b>=0xa0))return 0;
        return 3;
    }
    if(lead>=0xf0&&lead<=0xf4){
        if(value.size()-offset<4)return 0;
        const auto b=static_cast<unsigned char>(value[offset+1]);
        if((b&0xc0)!=0x80||(static_cast<unsigned char>(value[offset+2])&0xc0)!=0x80||(static_cast<unsigned char>(value[offset+3])&0xc0)!=0x80)return 0;
        if((lead==0xf0&&b<0x90)||(lead==0xf4&&b>=0x90))return 0;
        return 4;
    }
    return 0;
}
inline std::string bounded_utf8(const std::string &value,std::size_t maximum){
    std::size_t offset=0;
    while(offset<value.size()&&offset<maximum){const auto width=utf8_width(value,offset);if(!width||offset+width>maximum)break;offset+=width;}
    return value.substr(0,offset);
}
// DF strings are CP437, not UTF-8. Each input byte expands to at least one
// output byte, so taking at most maximum input bytes cannot omit a fitting
// output scalar. Bound conversion allocation before truncating at a UTF-8 edge.
inline std::string bounded_df_utf8(const std::string &value,std::size_t maximum){
    return bounded_utf8(DF2UTF(value.substr(0,maximum)),maximum);
}
struct SkillRecord{std::int32_t id,nominal,effective,experience;std::string key;};

inline std::uint32_t capture(std::uint32_t maximum,std::size_t maximum_bytes,std::string &out){
    if(maximum==0||maximum>4096||maximum_bytes<1024||maximum_bytes>16*1024*1024)return 3;
    std::vector<df::unit *> citizens;
    if(!Units::getCitizens(citizens,true,false))return 5;
    if(citizens.size()>maximum)return 3;
    // A strict complete roster cannot silently discard an invalid member.
    // Check before sorting, since the comparator dereferences every pointer.
    if(std::find(citizens.begin(),citizens.end(),nullptr)!=citizens.end())return 5;
    std::sort(citizens.begin(),citizens.end(),[](const df::unit *a,const df::unit *b){return a->id<b->id;});
    if(citizens.size()>std::numeric_limits<std::uint32_t>::max())return 5;
    out.assign("DFMC1800",8);base::u32(out,static_cast<std::uint32_t>(citizens.size()));
    std::int32_t previous=-1;std::size_t skill_total=0;
    for(auto *unit:citizens){
        if(!unit||unit->id<0||unit->id<=previous||!Units::isCitizen(unit,false)||Units::isResident(unit,false))return 5;
        const auto *visible=Units::getVisibleName(unit);
        const auto name=bounded_df_utf8(visible?Translation::translateName(visible,false):std::string(),256);
        const auto race=bounded_df_utf8(Units::getRaceReadableName(unit),128);
        if(!base::utf8(name,256,true)||!base::utf8(race,128,true))return 5;
        const auto profession=static_cast<std::int32_t>(Units::getProfession(unit));
        const auto stress=Units::getStressCategory(unit);
        if(profession<0||stress<0||stress>6)return 5;
        const auto position=Units::getPosition(unit);
        const bool available_preserve_social=Units::isJobAvailable(unit,true);
        const bool available_interrupt_social=Units::isJobAvailable(unit,false);
        std::uint16_t flags=0;unsigned bit=0;
        for(bool value:{Units::isAlive(unit),Units::isSane(unit),Units::isActive(unit),Units::isVisible(unit),true,false,
            Units::isBaby(unit),Units::isChild(unit),Units::isAdult(unit)}){if(value)flags|=static_cast<std::uint16_t>(1u<<bit);++bit;}
        std::vector<SkillRecord> skills;skills.reserve(32);
        for(std::int32_t raw=0;raw<=static_cast<std::int32_t>(ENUM_LAST_ITEM(job_skill));++raw){
            const auto skill=static_cast<df::job_skill>(raw);if(!is_valid_enum_item(skill))continue;
            const auto nominal=Units::getNominalSkill(unit,skill,true);const auto effective=Units::getEffectiveSkill(unit,skill);
            const auto experience=Units::getExperience(unit,skill,false);
            if(nominal<0||effective<0||experience<0)return 5;
            if(nominal==0&&effective==0&&experience==0)continue;
            const std::string key=ENUM_KEY_STR(job_skill,skill);if(!base::utf8(key,MAX_SKILL_KEY_BYTES)||skills.size()>=MAX_SKILLS_PER_CITIZEN||skill_total>=MAX_SKILLS_TOTAL)return 3;
            skills.push_back({raw,nominal,effective,experience,key});++skill_total;
        }
        base::u32(out,static_cast<std::uint32_t>(unit->id));base::text(out,name);base::text(out,race);base::i32(out,profession);
        base::i32(out,position.x);base::i32(out,position.y);base::i32(out,position.z);base::u16(out,flags);base::i32(out,stress);
        out.push_back(available_preserve_social?1:0);out.push_back(available_interrupt_social?1:0);base::u16(out,static_cast<std::uint16_t>(skills.size()));
        for(const auto &skill:skills){base::i32(out,skill.id);base::text(out,skill.key);base::i32(out,skill.nominal);base::i32(out,skill.effective);base::i32(out,skill.experience);}
        if(out.size()>maximum_bytes)return 3;
        previous=unit->id;
    }
    return out.size()>maximum_bytes?3:0;
}
}
