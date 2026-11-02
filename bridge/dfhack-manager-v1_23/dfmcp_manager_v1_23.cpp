#include "../common/manager_appointment.h"
#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <memory>
#include <set>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/Gui.h"
#include "modules/Maps.h"
#include "modules/Units.h"
#include "modules/World.h"
#include "df/entity_position.h"
#include "df/entity_position_assignment.h"
#include "df/entity_position_profile_claimst.h"
#include "df/entity_position_flags.h"
#include "df/global_objects.h"
#include "df/gamest.h"
#include "df/historical_entity.h"
#include "df/historical_figure.h"
#include "df/histfig_entity_link_memberst.h"
#include "df/histfig_entity_link_positionst.h"
#include "df/histfig_entity_link_type.h"
#include "df/history_event.h"
#include "df/history_event_add_hf_entity_linkst.h"
#include "df/history_event_remove_hf_entity_linkst.h"
#include "df/history_event_type.h"
#include "df/nemesis_record.h"
#include "df/plotinfost.h"
#include "df/unit.h"
#include "df/viewscreen_dwarfmodest.h"
#include "df/world.h"
#include "DfmcpManagerV1_23.pb.h"
using namespace DFHack;
namespace ma=dfmcp_manager;
namespace wire=dfmcp::manager::v1_23;
DFHACK_PLUGIN("dfmcp_manager_v1_23");
namespace {
std::uint64_t now_ms() {
    const auto n=std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch()).count();
    ma::require(n>=0,5);return static_cast<std::uint64_t>(n);
}
ma::Engine engine(now_ms()|1);
struct Work {
    std::uint64_t deadline;
    explicit Work(std::uint32_t millis):deadline(now_ms()+millis){ma::require(millis>0 && millis<=60000,5);}
    void check() const {ma::require(now_ms()<deadline,5);}
};
bool enabled(const char *name){const char *v=std::getenv(name);return v && std::string_view(v)=="1";}
bool development(){return !std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") && enabled("DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23");}
void credential(const std::string &provided) {
    const char *raw=std::getenv("DFMCP_MANAGER_V1_23_TOKEN");const std::string_view expected=raw?raw:"";
    ma::require(expected.size()>=32 && expected.size()<=256 && provided.size()>=32 && provided.size()<=256,1);
    std::size_t difference=expected.size()^provided.size();
    for(std::size_t i=0;i<256;++i)difference|=(i<expected.size()?static_cast<unsigned char>(expected[i]):0)^(i<provided.size()?static_cast<unsigned char>(provided[i]):0);
    ma::require(!difference,1);
}
template<class Flags> std::string flags(const Flags &f) {
    ma::require(f.size()>=0 && f.size()<=16 && (!f.size() || f.bits()),5);
    return f.size()?std::string(reinterpret_cast<const char*>(f.bits()),static_cast<std::size_t>(f.size())):std::string();
}
template<class Values> void integers(std::string &out,const Values &v) {
    ma::require(v.size()<=256,5);ma::u16(out,static_cast<std::uint16_t>(v.size()));for(auto n:v)ma::i32(out,n);
}
template<class Values> void strings(std::string &out,const Values &v) {
    ma::require(v.size()<=256,5);ma::u16(out,static_cast<std::uint16_t>(v.size()));
    for(auto *s:v){ma::require(s && s->size()<=512,5);ma::field(out,*s);}
}
std::string position(const df::entity_position &p) {
    ma::require(dfmcp_workshop::utf8(p.code,128),5);std::string out;ma::i32(out,p.id);ma::field(out,p.code);ma::field(out,flags(p.flags));
    for(auto n:{static_cast<int>(p.squad_size),static_cast<int>(p.land_holder),static_cast<int>(p.requires_population),
        static_cast<int>(p.execution_skill),p.precedence,p.replaced_by,static_cast<int>(p.number),p.required_boxes,p.required_cabinets,
        p.required_racks,p.required_stands,p.required_office,p.required_bedroom,p.required_dining,p.required_tomb,p.mandate_max,p.demand_max,p.best_appointment_precedence})ma::i32(out,n);
    integers(out,p.allowed_creature);integers(out,p.rejected_creature);strings(out,p.allowed_class);strings(out,p.rejected_class);
    integers(out,p.commander_id);integers(out,p.commander_civ);integers(out,p.commander_types);
    integers(out,p.appointed_by);integers(out,p.appointed_by_civ);integers(out,p.succession_by_position);
    for(bool b:p.responsibilities)ma::flag(out,b);
    for(const auto *names:{&p.name,&p.name_female,&p.name_male,&p.spouse,&p.spouse_female,&p.spouse_male,&p.squad})
        for(const auto &s:*names){ma::require(s.size()<=512,5);ma::field(out,s);}
    for(const auto *s:{&p.land_name,&p.description}){ma::require(s->size()<=1024,5);ma::field(out,*s);}
    for(auto n:p.color)ma::i32(out,n);
    return out;
}
ma::Assignment assignment(const df::entity_position_assignment &a) {
    ma::Assignment out;out.id=a.id;out.position=a.position_id;out.figure=a.histfig;out.last_figure=a.histfig2;out.cache=a.position_vector_idx;
    out.flags=flags(a.flags);out.squad=a.squad_id;out.site=a.st_id;out.abstract_building=a.ab_id;
    out.vassal_entity=a.vassal_of_entity_id;out.vassal_assignment=a.vassal_of_position_profile_id;out.temp=a.temp;
    ma::require(a.claim.size()<=32,5);for(const auto *c:a.claim){ma::require(c,5);out.claims.push_back(c->hfid);}
    // assigned_army_controller_id is documented uninitialized/not saved; never read it.
    return out;
}
df::historical_entity *entity(std::int32_t id) {
    auto *e=df::historical_entity::find(id);ma::require(e && e->id==id,5);return e;
}
ma::Entity entity_view(df::historical_entity *e,const Work &work) {
    const auto &all=df::global::world->entities.all;ma::require(all.size()<=65536,5);ma::Entity out;out.id=e->id;
    for(std::size_t i=0;i<all.size();++i){work.check();ma::require(all[i],5);if(all[i]==e){ma::require(out.index==-1,5);out.index=static_cast<std::int32_t>(i);}}
    ma::require(out.index>=0,5);out.next_position=e->positions.next_position_id;out.next_assignment=e->positions.next_assignment_id;
    ma::require(e->positions.own.size()<=ma::MAX_POSITIONS && e->positions.assignments.size()<=ma::MAX_ASSIGNMENTS,5);
    std::string defs;std::int32_t previous=-1;
    for(const auto *p:e->positions.own){work.check();ma::require(p && p->id>previous,5);previous=p->id;ma::field(defs,position(*p));}
    out.positions_digest=ma::hash("dfmcp-manager-position-definitions/1",defs);
    for(const auto *a:e->positions.assignments){work.check();ma::require(a,5);out.assignments.push_back(assignment(*a));}
    ma::require(e->assignments_by_type.size()<=ma::MAX_ROLE_BUCKETS,5);
    for(const auto &bucket:e->assignments_by_type){
        work.check();ma::require(bucket.size()<=ma::MAX_ROLE_ENTRIES,5);
        std::vector<std::int32_t> ids;
        for(const auto *a:bucket){
            work.check();ma::require(a,5);
            ma::require(std::find(e->positions.assignments.begin(),e->positions.assignments.end(),a)!=e->positions.assignments.end(),5);
            ma::require(std::find(ids.begin(),ids.end(),a->id)==ids.end(),5);ids.push_back(a->id);
        }
        out.role_buckets.push_back(std::move(ids));
    }
    return out;
}
bool ui_quiescent() {
    // Use the actual top screen, not a skipped underlying screen or cached focus.
    if(!df::global::game)return false;
    // Closed cached candidate arrays do not establish a pending UI effect.
    // The initial profile requires the entire actual info panel closed.
    if(df::global::game->main_interface.info.open)return false;
    auto *top=Gui::getCurViewscreen(false);
    if(!top || !strict_virtual_cast<df::viewscreen_dwarfmodest>(top))return false;
    const auto focus=Gui::getFocusStrings(top);
    return focus.size()==1 && focus[0]=="dwarfmode/Default";
}
ma::Event event_view(df::history_event *e) {
    ma::require(e,5);ma::Event out;out.id=e->id;out.year=e->year;out.seconds=e->seconds;out.type=static_cast<std::int32_t>(e->getType());out.flags=flags(e->flags);
    if(auto *a=strict_virtual_cast<df::history_event_add_hf_entity_linkst>(e)){
        if(a->link_type==df::histfig_entity_link_type::POSITION){out.position=true;out.added=true;out.entity=a->civ;out.figure=a->histfig;out.position_id=a->position_id;out.appointer=a->appointer_hfid;out.promise=a->promise_to_hfid;}
    } else if(auto *r=strict_virtual_cast<df::history_event_remove_hf_entity_linkst>(e)){
        if(r->link_type==df::histfig_entity_link_type::POSITION){out.position=true;out.entity=r->civ;out.figure=r->histfig;out.position_id=r->position_id;}
    }
    return out;
}
ma::Capture capture(const ma::Selection &selected,const Work &work) {
    work.check();ma::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded() && World::isFortressMode()
        && df::global::world && df::global::plotinfo && df::global::hist_event_next_id,4);
    auto *world=df::global::world;ma::Capture out;out.minor=23;out.selection=selected;out.paused=World::ReadPauseState();out.ui_quiescent=ui_quiescent();
    const auto year=World::ReadCurrentYear();const auto tick=World::ReadCurrentTick();const auto site=World::GetCurrentSiteId();
    ma::require(year>=0 && tick<403200 && site>=0 && *df::global::hist_event_next_id>=0,5);
    out.year=static_cast<std::uint32_t>(year);out.year_tick=tick;out.tick=std::uint64_t{out.year}*403200+tick;
    out.site=static_cast<std::uint32_t>(site);out.folder=World::ReadWorldFolder();out.next_event=static_cast<std::uint32_t>(*df::global::hist_event_next_id);
    std::int32_t x=0,y=0,z=0;Maps::getTileSize(x,y,z);ma::require(x>0 && y>0 && z>0,5);
    out.dimensions={static_cast<std::uint32_t>(x),static_cast<std::uint32_t>(y),static_cast<std::uint32_t>(z)};
    auto *group=entity(df::global::plotinfo->group_id),*civ=entity(df::global::plotinfo->civ_id);
    out.group=entity_view(group,work);out.civ=entity_view(civ,work);
    std::string domain;std::set<std::int32_t> entities;
    for(const auto *e:world->entities.all){work.check();ma::require(e && e->id>=0 && entities.insert(e->id).second,5);ma::i32(domain,e->id);}
    out.entity_domain_digest=ma::hash("dfmcp-manager-entity-domain/1",domain);
    df::entity_position *manager=nullptr;
    for(std::size_t i=0;i<group->positions.own.size();++i)if(group->positions.own[i]->code=="MANAGER"){
        ma::require(!manager,4);manager=group->positions.own[i];out.manager_position=manager->id;out.manager_index=static_cast<std::int32_t>(i);
    }
    ma::require(manager,4);bool civ_manager=false;for(const auto *p:civ->positions.own)civ_manager=civ_manager || p->code=="MANAGER";
    unsigned responsibilities=0;for(bool value:manager->responsibilities)if(value)++responsibilities;
    out.role_eligible=responsibilities==1 && manager->responsibilities[ma::MANAGE_PRODUCTION]
        && !civ_manager && manager->number==1 && manager->requires_population==0 && manager->replaced_by==-1
        && manager->allowed_creature.empty() && manager->allowed_class.empty() && manager->rejected_creature.empty() && manager->rejected_class.empty()
        && !manager->flags.is_set(df::entity_position_flags::ELECTED) && !manager->flags.is_set(df::entity_position_flags::GENDER_MALE)
        && !manager->flags.is_set(df::entity_position_flags::GENDER_FEMALE) && !manager->flags.is_set(df::entity_position_flags::SUCCESSION_BY_HEIR);
    const auto *target=out.target();ma::require(target,4);
    auto *unit=df::unit::find(static_cast<std::int32_t>(selected.unit));
    auto *hf=df::historical_figure::find(static_cast<std::int32_t>(selected.figure));ma::require(unit && hf,4);
    auto *nemesis=Units::getNemesis(unit);
    out.identity=unit->hist_figure_id==hf->id && hf->unit_id==unit->id && nemesis && nemesis->figure==hf && unit->civ_id==civ->id;
    out.active=Units::isActive(unit);out.citizen=Units::isCitizen(unit);out.adult=Units::isAdult(unit);out.sane=Units::isSane(unit);
    ma::require(hf->entity_links.size()<ma::MAX_LINKS,5);bool group_member=false,civ_member=false;
    for(auto *link:hf->entity_links){
        work.check();ma::require(link,5);ma::Link l;l.type=static_cast<std::int32_t>(link->getType());l.entity=link->entity_id;l.entity_cache=link->entity_vector_idx;l.strength=link->link_strength;
        if(auto *p=strict_virtual_cast<df::histfig_entity_link_positionst>(link)){l.assignment=p->assignment_id;l.assignment_cache=p->assignment_vector_idx;l.start_year=p->start_year;}
        else ma::require(strict_virtual_cast<df::histfig_entity_link_memberst>(link),4);
        group_member=group_member || (l.type==0 && l.entity==group->id && l.strength>0);
        civ_member=civ_member || (l.type==0 && l.entity==civ->id && l.strength>0);out.links.push_back(l);
    }
    out.member=group_member && civ_member;
    ma::require(world->units.active.size()<=ma::MAX_UNITS,5);std::set<std::int32_t> unit_ids;std::string roster;unsigned selected_active=0;
    for(auto *u:world->units.active){work.check();ma::require(u && u->id>=0 && unit_ids.insert(u->id).second,5);if(u==unit)++selected_active;
        ma::i32(roster,u->id);ma::i32(roster,u->hist_figure_id);ma::i32(roster,u->civ_id);
        for(auto f:{u->flags1.whole,u->flags2.whole,u->flags3.whole})ma::u32(roster,f);
    }
    out.active=out.active && selected_active==1;out.roster_digest=ma::hash("dfmcp-manager-active-roster/1",roster);
    ma::require(world->history.figures.size()<=ma::MAX_FIGURES,5);std::set<std::int32_t> figure_ids;std::size_t links=0;
    for(const auto *f:world->history.figures){work.check();ma::require(f && f->id>=0 && figure_ids.insert(f->id).second && f->entity_links.size()<=ma::MAX_LINKS,5);
        links+=f->entity_links.size();ma::require(links<=1048576,5);
        for(auto *l:f->entity_links){work.check();ma::require(l,5);if(const auto *p=strict_virtual_cast<df::histfig_entity_link_positionst>(l))
            if(p->entity_id==group->id && p->assignment_id==target->id)++out.manager_links;}
    }
    ma::require(figure_ids.count(hf->id)==1,5);
    auto *lookup=Units::getUnitByNobleRole("MANAGER");out.lookup_unit=lookup?lookup->id:-1;
    std::vector<Units::NoblePosition> roles;Units::getNoblePositions(&roles,unit);unsigned managers=0,leaders=0;
    for(const auto &r:roles){ma::require(r.entity && r.assignment && r.position,5);
        if(r.entity==group && r.position->code=="MANAGER" && r.assignment->id==target->id)++managers;
        if(r.entity==group && r.position->code=="EXPEDITION_LEADER" && r.assignment->histfig==hf->id && r.assignment->histfig2==hf->id)++leaders;}
    out.expedition_leader=leaders==1 && Units::getUnitByNobleRole("EXPEDITION_LEADER")==unit;
    out.dual_getters=(target->figure==-1 && !lookup && managers==0 && out.manager_links==0)
        || (target->figure==hf->id && lookup==unit && managers==1 && out.manager_links==1);
    ma::require(world->history.events.size()<ma::MAX_HISTORY,5);out.history_digest=ma::hash("dfmcp-manager-history-projection/1","");std::int32_t previous=-1;
    for(auto *e:world->history.events){work.check();auto row=event_view(e);ma::require(row.id>previous && row.id<*df::global::hist_event_next_id,5);previous=row.id;
        out.history_digest=ma::extend_history(out.history_digest,row);out.last_event=row;}
    out.history_count=static_cast<std::uint32_t>(world->history.events.size());work.check();return out;
}
void appoint(const ma::Capture &before,const std::string &token,const Work &work) {
    auto read=[&](const ma::Selection &s){return capture(s,work);};
    auto unchanged=engine.inspect(before.selection,read);unchanged.sequence=before.sequence;
    ma::require(unchanged.encode()==before.encode() && unchanged.eligible(),6);
    auto *hf=df::historical_figure::find(static_cast<std::int32_t>(before.selection.figure));auto *group=entity(before.group.id);
    const auto *old=before.target();ma::require(hf && old,5);
    const auto index=static_cast<std::size_t>(old-before.group.assignments.data());ma::require(index<group->positions.assignments.size(),5);
    auto *a=group->positions.assignments[index];ma::require(a && a->id==old->id,6);
    std::unique_ptr<df::histfig_entity_link_positionst> link(df::allocate<df::histfig_entity_link_positionst>());
    std::unique_ptr<df::history_event_add_hf_entity_linkst> event(df::allocate<df::history_event_add_hf_entity_linkst>());
    ma::require(link && event && flags(event->flags)==before.appointment_event().flags,5);
    link->entity_id=before.group.id;link->entity_vector_idx=before.group.index;link->link_strength=100;
    link->assignment_id=old->id;link->assignment_vector_idx=static_cast<std::int32_t>(index);link->start_year=static_cast<std::int32_t>(before.year);
    event->id=static_cast<std::int32_t>(before.next_event);event->year=static_cast<std::int32_t>(before.year);event->seconds=static_cast<std::int32_t>(before.year_tick);
    event->civ=before.group.id;event->histfig=static_cast<std::int32_t>(before.selection.figure);event->link_type=df::histfig_entity_link_type::POSITION;
    event->position_id=before.manager_position;event->appointer_hfid=-1;event->promise_to_hfid=-1;
    hf->entity_links.reserve(hf->entity_links.size()+1);df::global::world->history.events.reserve(df::global::world->history.events.size()+1);
    auto &production=group->assignments_by_type[ma::MANAGE_PRODUCTION];
    ma::require(production.empty() && production.size()<ma::MAX_ROLE_ENTRIES,6);production.reserve(production.size()+1);
    unchanged=engine.inspect(before.selection,read);unchanged.sequence=before.sequence;
    ma::require(unchanged.encode()==before.encode() && unchanged.eligible(),6);
    ma::require(development() && enabled("DFMCP_MANAGER_V1_23_ALLOW_APPOINT"),1);credential(token);work.check();
    // Allocations, constructor checks, capacity growth and revalidation precede
    // every world write. No arbitrary role, displacement or order activation.
    production.push_back(a);
    a->histfig=static_cast<std::int32_t>(before.selection.figure);a->histfig2=a->histfig;a->position_vector_idx=before.manager_index;
    hf->entity_links.push_back(link.release());df::global::world->history.events.push_back(event.release());++*df::global::hist_event_next_id;
}
ma::Insertion verify(const ma::Capture &before,const Work &work) {
    auto now=engine.inspect(before.selection,[&](const ma::Selection&s){return capture(s,work);});
    ma::Insertion out;out.unit=before.selection.unit;out.figure=before.selection.figure;out.group=before.group.id;
    out.assignment=before.target()->id;out.event=static_cast<std::int32_t>(before.next_event);
    out.lookup=now.lookup_unit==static_cast<std::int32_t>(out.unit);out.noble=now.dual_getters;return out;
}
unsigned shape(const wire::Request *in){return (in->has_unit_id()?1:0)|(in->has_histfig_id()?2:0)|(in->has_idempotency_key()?4:0)|(in->has_expected_witness()?8:0)|(in->has_plan_digest()?16:0)|(in->has_prepare_token()?32:0);}
ma::Selection selection(const wire::Request *in){return {in->unit_id(),in->histfig_id()};}
void empty(const wire::Request *in,wire::Reply *out,std::uint32_t code) {
    out->Clear();out->set_accepted(false);out->set_failure_code(code);out->set_client_nonce(in->client_nonce().size()<=64?in->client_nonce():std::string());
    out->set_protocol_major(1);out->set_protocol_minor(23);out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
}
template<class Body> command_result guarded(const wire::Request *in,wire::Reply *out,unsigned mask,bool write,Body body) {
    try{
        empty(in,out,3);ma::require(in->IsInitialized() && in->ByteSize()<=2048 && in->GetReflection()->GetUnknownFields(*in).empty(),3);
        ma::require(in->protocol_major()==1 && in->protocol_minor()==23,2);ma::require(in->client_nonce().size()>=16 && in->client_nonce().size()<=64,3);
        ma::require(shape(in)==mask && development(),1);credential(in->bearer_token());if(write)ma::require(enabled("DFMCP_MANAGER_V1_23_ALLOW_APPOINT"),1);
        Work work(in->max_wall_millis());CoreSuspender suspended;work.check();
        const auto &info=Core::getInstance().vinfo;const auto version=info?info->getVersion():std::string();const char *dfhack=Version::dfhack_version();
        ma::require(engine.available() && dfmcp_workshop::utf8(version,128) && dfhack && dfmcp_workshop::utf8(dfhack,128),5);
        out->set_bridge_generation(engine.generation());out->set_df_version(version);out->set_dfhack_version(dfhack);body(work);
        out->set_unresolved(engine.unresolved());out->set_retained_records(engine.size());out->set_failure_code(0);out->set_accepted(true);
        ma::require(out->ByteSize()<=256*1024,5);return CR_OK;
    }catch(const dfmcp_workshop::Failure &e){empty(in,out,e.code);return CR_OK;}catch(...){empty(in,out,5);return CR_OK;}
}
command_result Handshake(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,0,false,[](const Work&){});}
command_result ReadManager(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,3,false,[&](const Work&w){out->set_observation(engine.inspect(selection(in),[&](const ma::Selection&s){return capture(s,w);}).encode());});}
command_result PrepareManager(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,31,true,[&](const Work&w){
    const bool replay=engine.query(in->idempotency_key(),in->plan_digest())!=nullptr;out->set_effect_record(engine.prepare(in->idempotency_key(),selection(in),in->expected_witness(),in->plan_digest(),now_ms(),[&](const ma::Selection&s){return capture(s,w);}).encode());out->set_replayed(replay);});}
command_result CommitManager(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,52,true,[&](const Work&w){
    out->set_effect_record(engine.commit(in->idempotency_key(),in->plan_digest(),in->prepare_token(),now_ms(),[&](const ma::Selection&s){return capture(s,w);},[&](const ma::Capture&b){appoint(b,in->bearer_token(),w);},[&](const ma::Capture&b){return verify(b,w);}).encode());});}
command_result QueryManager(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,20,false,[&](const Work&w){
    if(const auto *r=engine.reconcile(in->idempotency_key(),in->plan_digest(),[&](const ma::Selection&s){return capture(s,w);},[&](const ma::Capture&b){return verify(b,w);}))out->set_effect_record(r->encode());});}
command_result CancelManager(color_ostream&,const wire::Request *in,wire::Reply *out){return guarded(in,out,52,false,[&](const Work&){out->set_effect_record(engine.cancel(in->idempotency_key(),in->plan_digest(),in->prepare_token()).encode());});}
} // namespace
DFhackCExport command_result plugin_init(color_ostream&,std::vector<PluginCommand>&){return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream&){engine.change_source();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream&,state_change_event e){if(e==SC_MAP_LOADED || e==SC_MAP_UNLOADED || e==SC_WORLD_LOADED || e==SC_WORLD_UNLOADED)engine.change_source();else if(e==SC_PAUSED || e==SC_UNPAUSED)engine.interrupt();return CR_OK;}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream&){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("ReadManager",ReadManager,0);s->addFunction("PrepareManager",PrepareManager,0);s->addFunction("CommitManager",CommitManager,0);s->addFunction("QueryManager",QueryManager,0);s->addFunction("CancelManager",CancelManager,0);return s;}
