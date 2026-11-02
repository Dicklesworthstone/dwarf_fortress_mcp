#pragma once
#include "workshop_construction.h"

// Manager/1.22. Value evidence only: no addresses, arbitrary roles or commands.
// DFHack objects are handled by the separately tested SDK transaction shell.
namespace dfmcp_manager {
using dfmcp_workshop::require;
using dfmcp_workshop::u16;
using dfmcp_workshop::u32;
using dfmcp_workshop::i32;
using dfmcp_workshop::u64;
using dfmcp_workshop::field;
using dfmcp_workshop::flag;
using dfmcp_workshop::hash;
using dfmcp_workshop::key_valid;
constexpr std::size_t MAX_CAPTURE_BYTES=65536, MAX_RECORD_BYTES=2*MAX_CAPTURE_BYTES+1024;
constexpr std::size_t MAX_ASSIGNMENTS=256, MAX_POSITIONS=256, MAX_LINKS=256;
constexpr std::size_t MAX_HISTORY=65536, MAX_UNITS=8192, MAX_FIGURES=65536;
struct Selection {
    std::uint32_t unit=0, figure=0;
    std::string encode() const {
        require(unit<INT32_MAX && figure<INT32_MAX,5);
        std::string out;u32(out,unit);u32(out,figure);return out;
    }
};
struct Assignment {
    std::int32_t id=-1, position=-1, figure=-1, last_figure=-1, cache=-1;
    std::int32_t squad=-1, site=-1, abstract_building=-1, vassal_entity=-1, vassal_assignment=-1, temp=0;
    std::string flags;
    std::vector<std::int32_t> claims;
    bool ordinary_active() const {
        // Known SDK bits: active, temp, temp2, temp3. Preserve every raw byte.
        return !flags.empty() && (static_cast<unsigned char>(flags.front()) & 0x0f) == 1;
    }
    std::string encode() const {
        require(id>=0 && position>=0 && flags.size()<=16 && claims.size()<=32,5);
        std::string out;
        for(auto n:{id,position,figure,last_figure,cache,squad,site,abstract_building,vassal_entity,vassal_assignment,temp})i32(out,n);
        field(out,flags);u16(out,static_cast<std::uint16_t>(claims.size()));for(auto n:claims)i32(out,n);
        return out;
    }
};
struct Entity {
    std::int32_t id=-1, index=-1, next_position=-1, next_assignment=-1;
    // Complete ordered position definitions under a declared stable field projection.
    std::string positions_digest;
    std::vector<Assignment> assignments;
    std::string encode() const {
        require(id>=0 && index>=0 && next_position>=0 && next_assignment>=0
            && positions_digest.size()==32 && assignments.size()<=MAX_ASSIGNMENTS,5);
        std::string out;for(auto n:{id,index,next_position,next_assignment})i32(out,n);out+=positions_digest;
        u16(out,static_cast<std::uint16_t>(assignments.size()));std::int32_t previous=-1;
        for(const auto &a:assignments){require(a.id>previous,5);previous=a.id;out+=a.encode();}
        return out;
    }
};
struct Link {
    // Closed to ordinary MEMBER and POSITION links. Unknown subclasses refuse.
    std::int32_t type=0, entity=-1, entity_cache=-1, strength=0;
    std::int32_t assignment=-1, assignment_cache=-1, start_year=-1;
    std::string encode() const {
        require((type==0 || type==10) && entity>=0 && strength>=0 && strength<=INT16_MAX,5);
        require(type==10 || (assignment==-1 && assignment_cache==-1 && start_year==-1),5);
        std::string out;for(auto n:{type,entity,entity_cache,strength,assignment,assignment_cache,start_year})i32(out,n);return out;
    }
};
struct Event {
    std::int32_t id=-1, year=-1, seconds=-1, type=-1;
    std::string flags;
    // Full fields of POSITION add/remove events; defaults for other subtypes.
    bool position=false, added=false;
    std::int32_t entity=-1, figure=-1, position_id=-1, appointer=-1, promise=-1;
    std::string encode() const {
        require(id>=0 && flags.size()<=16,5);
        std::string out;for(auto n:{id,year,seconds,type})i32(out,n);field(out,flags);flag(out,position);flag(out,added);
        for(auto n:{entity,figure,position_id,appointer,promise}){i32(out,n);}
        return out;
    }
};
inline std::string extend_history(const std::string &prior,const Event &event) {
    require(prior.size()==32,5);return hash("dfmcp-manager-history-projection/1",prior+event.encode());
}
struct Capture {
    std::uint64_t generation=0, sequence=0, tick=0;
    std::uint32_t site=0, year=0, year_tick=0;
    std::array<std::uint32_t,3> dimensions{};
    std::string folder;
    Selection selection;
    Entity group,civ;
    std::int32_t manager_position=-1, manager_index=-1, lookup_unit=-1;
    std::uint32_t manager_links=0, next_event=0, history_count=0;
    std::string history_digest, roster_digest, entity_domain_digest;
    std::optional<Event> last_event;
    std::vector<Link> links;
    bool paused=false, active=false, citizen=false, adult=false, sane=false;
    bool identity=false, member=false, role_eligible=false, dual_getters=false;
    bool expedition_leader=false, ui_quiescent=false;
    std::string encode() const {
        require(generation && generation<UINT64_MAX && sequence<UINT64_MAX
            && year_tick<403200 && tick==std::uint64_t{year}*403200+year_tick
            && dfmcp_workshop::utf8(folder,512) && site<INT32_MAX && manager_links<=1
            && next_event<INT32_MAX && history_count<=MAX_HISTORY && links.size()<=MAX_LINKS
            && history_digest.size()==32 && roster_digest.size()==32 && entity_domain_digest.size()==32,5);
        std::string out="DFMMC022";for(auto n:{generation,sequence,tick})u64(out,n);
        for(auto n:{site,year,year_tick}){u32(out,n);}
        for(auto n:dimensions){require(n && n<=32768,5);u32(out,n);}
        field(out,folder);out+=selection.encode();out+=group.encode();out+=civ.encode();
        for(auto n:{manager_position,manager_index,lookup_unit})i32(out,n);
        for(auto n:{manager_links,next_event,history_count})u32(out,n);
        out+=history_digest;out+=roster_digest;out+=entity_domain_digest;
        flag(out,last_event.has_value());if(last_event)out+=last_event->encode();
        u16(out,static_cast<std::uint16_t>(links.size()));for(const auto &l:links)out+=l.encode();
        for(bool v:{paused,active,citizen,adult,sane,identity,member,role_eligible,dual_getters,expedition_leader,ui_quiescent})flag(out,v);
        require(out.size()<=MAX_CAPTURE_BYTES,5);return out;
    }
    const Assignment *target() const {
        const Assignment *found=nullptr;
        for(const auto &a:group.assignments)if(a.position==manager_position){if(found)return nullptr;found=&a;}
        return found;
    }
    bool eligible() const {
        (void)encode();const auto *a=target();
        return paused && active && citizen && adult && sane && identity && member && role_eligible && expedition_leader && ui_quiescent
            && dual_getters && lookup_unit==-1 && manager_links==0 && group.id!=civ.id
            && a && a->ordinary_active() && a->figure==-1 && a->last_figure==-1 && a->cache==-1 && a->claims.empty()
            && a->squad==-1 && a->site==-1 && a->abstract_building==-1
            && a->vassal_entity==-1 && a->vassal_assignment==-1 && a->temp==0
            && manager_position>=0 && manager_index>=0
            && next_event+1<INT32_MAX && history_count<MAX_HISTORY && sequence<UINT64_MAX-1;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    Event appointment_event() const {
        Event e;e.id=static_cast<std::int32_t>(next_event);e.year=static_cast<std::int32_t>(year);
        e.seconds=static_cast<std::int32_t>(year_tick);e.type=4;e.position=true;e.added=true;
        // history_event's generated constructor allocates a zeroed BitArray
        // for the three named flags. Q observed all three false on UI commit.
        e.flags=std::string(1,'\0');
        e.entity=group.id;e.figure=static_cast<std::int32_t>(selection.figure);e.position_id=manager_position;return e;
    }
    Capture expected_after() const {
        require(eligible(),4);auto out=*this;++out.sequence;
        for(auto &a:out.group.assignments)if(a.position==manager_position){a.figure=static_cast<std::int32_t>(selection.figure);a.last_figure=a.figure;a.cache=manager_index;}
        Link link;link.type=10;link.entity=group.id;link.entity_cache=group.index;link.strength=100;
        link.assignment=target()->id;link.assignment_cache=static_cast<std::int32_t>(target()-group.assignments.data());
        link.start_year=static_cast<std::int32_t>(year);out.links.push_back(link);
        out.last_event=appointment_event();out.history_digest=extend_history(history_digest,*out.last_event);
        ++out.history_count;++out.next_event;out.manager_links=1;out.lookup_unit=static_cast<std::int32_t>(selection.unit);
        return out;
    }
};
struct Insertion {
    std::uint32_t unit=0, figure=0;
    std::int32_t group=-1, assignment=-1, event=-1;
    bool lookup=false, noble=false;
    bool matches(const Capture &c) const {
        return unit==c.selection.unit && figure==c.selection.figure && group==c.group.id
            && c.target() && assignment==c.target()->id && event==static_cast<std::int32_t>(c.next_event) && lookup && noble;
    }
    std::string encode() const {
        std::string out="DFMMI022";u32(out,unit);u32(out,figure);for(auto n:{group,assignment,event})i32(out,n);flag(out,lookup);flag(out,noble);return out;
    }
};
enum class Phase:unsigned char {Prepared=0,Indeterminate=1,Placed=2,Refused=3,Cancelled=4};
enum class Reason:unsigned char {None=0,Stale=1,Expired=2,SourceChanged=3,Cancelled=4,NativeFailure=5};
inline std::string plan_digest(const Selection &s,const std::string &w){require(w.size()==32);return hash("dfmcp-manager-plan/1",s.encode()+w);}
inline std::string token_for(const std::string &k,const std::string &p){key_valid(k);require(p.size()==32);std::string out;field(out,k);return hash("dfmcp-manager-token/1",out+p).substr(0,16);}
struct Progress {};
struct Record {
    std::string key,plan,token;Capture before;std::optional<Capture> after;std::optional<Insertion> insertion;
    Phase phase=Phase::Prepared;Reason reason=Reason::None;std::uint64_t created_ms=0;
    std::string encode() const {
        key_valid(key);require(plan.size()==32 && token.size()==16,5);std::string out="DFMMR022";field(out,key);out+=plan;out+=token;
        const auto b=before.encode();u32(out,static_cast<std::uint32_t>(b.size()));out+=b;out.push_back(static_cast<char>(phase));out.push_back(static_cast<char>(reason));
        flag(out,after.has_value());if(after){const auto a=after->encode();u32(out,static_cast<std::uint32_t>(a.size()));out+=a;}
        flag(out,insertion.has_value());if(insertion)out+=insertion->encode();out+=hash("dfmcp-manager-receipt/1",out);require(out.size()<=MAX_RECORD_BYTES,5);return out;
    }
};
struct Model {
    using Capture=dfmcp_manager::Capture;using Selection=dfmcp_manager::Selection;using Progress=dfmcp_manager::Progress;
    using Record=dfmcp_manager::Record;using Phase=dfmcp_manager::Phase;using Reason=dfmcp_manager::Reason;
    static constexpr std::size_t MAX_RECORDS=64;static constexpr std::uint64_t PREPARE_MS=60000;
    static void require(bool v,std::uint32_t c=3){dfmcp_manager::require(v,c);}
    static void key_valid(const std::string &k){dfmcp_manager::key_valid(k);}
    static std::string plan_digest(const Selection &s,const std::string &w){return dfmcp_manager::plan_digest(s,w);}
    static std::string token_for(const std::string &k,const std::string &p){return dfmcp_manager::token_for(k,p);}
};
using Engine=dfmcp_one_attempt::Engine<Model>;
} // namespace dfmcp_manager
