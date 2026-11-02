#pragma once
// Explicit SDK/protobuf component doubles, never DFHack ABI or live-game proof.
#include <array>
#include <chrono>
#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <type_traits>
#include <vector>
namespace mock {
inline bool loaded=true,mapped=true,fortress=true,paused=true,initialized=true,unknown=false,suspended=false;
inline unsigned allocations=0,fail_allocation=0,world_reads=0,delay_millis=0;
inline bool getter_failure=false;
inline std::int32_t year=30,site=38,next_event=1992;
inline std::uint32_t tick=16800;
inline std::vector<std::string> focus={"dwarfmode/Default"};
inline std::function<void()> read_hook;
}
namespace df {
template<class T>T *allocate();
struct Flags {std::vector<unsigned char> data;int size()const{return static_cast<int>(data.size());}unsigned char *bits()const{return const_cast<unsigned char*>(data.data());}bool is_set(int i)const{return i>=0 && static_cast<unsigned>(i)/8<data.size() && (data[static_cast<unsigned>(i)/8]&(1<<(i%8)));}};
enum entity_position_flags {ELECTED=1,GENDER_MALE=4,GENDER_FEMALE=5,SUCCESSION_BY_HEIR=6};
enum class histfig_entity_link_type {MEMBER=0,POSITION=10,OTHER=99};
struct histfig_entity_link {int entity_id=-1,entity_vector_idx=-1;std::int16_t link_strength=0;virtual ~histfig_entity_link()=default;virtual histfig_entity_link_type getType(){return histfig_entity_link_type::OTHER;}};
struct histfig_entity_link_memberst:histfig_entity_link {histfig_entity_link_type getType()override{return histfig_entity_link_type::MEMBER;}};
struct histfig_entity_link_positionst:histfig_entity_link {
 int assignment_id=-1,assignment_vector_idx=-1,start_year=-1;
 histfig_entity_link_type getType()override{return histfig_entity_link_type::POSITION;}
protected:histfig_entity_link_positionst()=default;template<class T>friend T *allocate();
};
struct entity_position_profile_claimst {int hfid=-1;};
struct entity_position_assignment {int id=-1,histfig=-1,histfig2=-1,position_id=-1,position_vector_idx=-1;Flags flags;int squad_id=-1,st_id=-1,ab_id=-1,vassal_of_entity_id=-1,vassal_of_position_profile_id=-1,temp=0;std::vector<entity_position_profile_claimst*> claim;};
struct entity_position {
 std::string code;int id=-1;Flags flags;
 std::vector<int> allowed_creature,rejected_creature,commander_id,commander_civ,appointed_by,appointed_by_civ,succession_by_position;
 std::vector<std::int16_t> commander_types;std::vector<std::string*>allowed_class,rejected_class;
 std::array<std::string,2> name,name_female,name_male,spouse,spouse_female,spouse_male,squad;
 std::string land_name,description;std::array<std::int16_t,3> color{};std::array<bool,43> responsibilities{};
 std::int16_t squad_size=0,land_holder=0,requires_population=0,execution_skill=0,number=1;
 int precedence=30001,replaced_by=-1,required_boxes=0,required_cabinets=0,required_racks=0,required_stands=0,required_office=1,required_bedroom=0,required_dining=0,required_tomb=0,mandate_max=0,demand_max=0,best_appointment_precedence=30001;
};
struct historical_entity {
 int id=-1;struct Positions {int next_position_id=0,next_assignment_id=0;std::vector<entity_position*>own;std::vector<entity_position_assignment*>assignments;}positions;
 std::array<std::vector<entity_position_assignment*>,43> assignments_by_type;
 static historical_entity *find(int);
};
struct historical_figure {int id=-1,unit_id=-1;std::vector<histfig_entity_link*>entity_links;static historical_figure *find(int);};
struct nemesis_record {historical_figure *figure=nullptr;};
struct unit {int id=-1,hist_figure_id=-1,civ_id=-1;struct Flag {std::uint32_t whole=0;}flags1,flags2,flags3;bool active=true,citizen=true,adult=true,sane=true;nemesis_record nemesis;static unit *find(int);};
enum class history_event_type {OTHER=0,ADD_HF_ENTITY_LINK=4,REMOVE_HF_ENTITY_LINK=5};
// SDK static.ctors.inc initializes flags(ENUM_LAST_ITEM(history_event_flags)).
// The last enum is 2: BitArray allocates one zero byte, rather than no storage.
struct history_event {int year=-1,seconds=-1,id=-1;Flags flags{{0}};virtual ~history_event()=default;virtual history_event_type getType(){return history_event_type::OTHER;}};
struct history_event_add_hf_entity_linkst:history_event {
 int civ=-1,histfig=-1,position_id=-1,appointer_hfid=-1,promise_to_hfid=-1;histfig_entity_link_type link_type=histfig_entity_link_type::MEMBER;
 history_event_type getType()override{return history_event_type::ADD_HF_ENTITY_LINK;}
protected:history_event_add_hf_entity_linkst()=default;template<class T>friend T *allocate();
};
struct history_event_remove_hf_entity_linkst:history_event {int civ=-1,histfig=-1,position_id=-1;histfig_entity_link_type link_type=histfig_entity_link_type::MEMBER;history_event_type getType()override{return history_event_type::REMOVE_HF_ENTITY_LINK;}};
template<class T>T *allocate(){++mock::allocations;if(mock::fail_allocation==mock::allocations)return nullptr;return new T;}
struct viewscreen {virtual ~viewscreen()=default;};struct viewscreen_dwarfmodest:viewscreen{};
struct gamest {struct Main {struct Info {bool open=false;int current_mode=0;struct Administrators {bool choosing_candidate=false,assigning_symbol=false;std::vector<entity_position_assignment*>spec_prof;std::vector<int>spec_hfid,spec_enid;}administrators;}info;}main_interface;};
struct plotinfost {int group_id=35,civ_id=12;};
struct world {struct Entities {std::vector<historical_entity*>all;}entities;struct Units {std::vector<unit*>active;}units;struct History {std::vector<history_event*>events;std::vector<historical_figure*>figures;}history;};
namespace global {inline df::world *world=nullptr;inline df::plotinfost *plotinfo=nullptr;inline gamest *game=nullptr;inline std::int32_t *hist_event_next_id=&mock::next_event;}
inline historical_entity *historical_entity::find(int id){for(auto *e:global::world->entities.all)if(e&&e->id==id)return e;return nullptr;}
inline historical_figure *historical_figure::find(int id){for(auto *f:global::world->history.figures)if(f&&f->id==id)return f;return nullptr;}
inline unit *unit::find(int id){for(auto *u:global::world->units.active)if(u&&u->id==id)return u;return nullptr;}
}
template<class T,class U>T *strict_virtual_cast(U *p){static_assert(!std::is_const<U>::value,"SDK strict_virtual_cast accepts non-const virtual_ptr only");return dynamic_cast<T*>(p);}
namespace DFHack {
using std::string;
struct color_ostream{};enum command_result{CR_OK};struct PluginCommand{};
struct VersionInfo{std::string getVersion()const{return "53.16";}};
struct Core{std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();static Core &getInstance(){static Core c;return c;}bool isWorldLoaded()const{return mock::loaded;}bool isMapLoaded()const{return mock::mapped;}};
struct CoreSuspender {CoreSuspender(){if(mock::suspended)throw std::runtime_error("recursive suspender");mock::suspended=true;}~CoreSuspender(){mock::suspended=false;}};
struct RPCService{std::vector<std::string>names;template<class F>void addFunction(const char *n,F,int){names.push_back(n);}};
enum state_change_event{SC_MAP_LOADED,SC_MAP_UNLOADED,SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_PAUSED,SC_UNPAUSED};
namespace Version {inline const char *dfhack_version(){return "53.16-r3";}}
namespace World {
inline bool isFortressMode(){++mock::world_reads;if(!mock::suspended)throw std::runtime_error("unsuspended world read");if(mock::delay_millis)std::this_thread::sleep_for(std::chrono::milliseconds(mock::delay_millis));if(mock::read_hook)mock::read_hook();return mock::fortress;}
inline bool ReadPauseState(){return mock::paused;}inline int ReadCurrentYear(){return mock::year;}inline unsigned ReadCurrentTick(){return mock::tick;}inline int GetCurrentSiteId(){return mock::site;}inline std::string ReadWorldFolder(){return "region1";}
}
namespace Maps {inline void getTileSize(int &x,int &y,int &z){x=192;y=192;z=187;}}
namespace Gui {inline df::viewscreen *getCurViewscreen(bool){static df::viewscreen_dwarfmodest top;return &top;}inline std::vector<std::string>getFocusStrings(df::viewscreen*){return mock::focus;}}
namespace Units {
struct NoblePosition{df::historical_entity *entity=nullptr;df::entity_position_assignment *assignment=nullptr;df::entity_position *position=nullptr;};
inline bool isActive(df::unit*u){return u->active;}inline bool isCitizen(df::unit*u){return u->citizen;}inline bool isAdult(df::unit*u){return u->adult;}inline bool isSane(df::unit*u){return u->sane;}inline df::nemesis_record *getNemesis(df::unit*u){return &u->nemesis;}
inline df::unit *getUnitByNobleRole(std::string role){if(mock::getter_failure&&role=="MANAGER")return nullptr;
 for(int id:{df::global::plotinfo->civ_id,df::global::plotinfo->group_id}){auto *e=df::historical_entity::find(id);if(!e)continue;
 for(auto *p:e->positions.own)if(p&&p->code==role)for(auto *a:e->positions.assignments)if(a&&a->position_id==p->id){auto *f=df::historical_figure::find(a->histfig);if(f)if(auto *u=df::unit::find(f->unit_id))return u;}}
 return nullptr;}
inline bool getNoblePositions(std::vector<NoblePosition>*out,df::unit*u){out->clear();auto*f=df::historical_figure::find(u->hist_figure_id);if(!f)return false;
 for(auto*l:f->entity_links){if(auto*pos=strict_virtual_cast<df::histfig_entity_link_positionst>(l)){auto*e=df::historical_entity::find(pos->entity_id);if(!e)continue;for(auto*a:e->positions.assignments)if(a->id==pos->assignment_id)for(auto*p:e->positions.own)if(p->id==a->position_id)out->push_back({e,a,p});}}
 return !out->empty();}
}
}
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace dfmcp::manager::v1_23 {
struct Request {
 unsigned mask=0;std::string bearer=std::string(32,'s'),nonce=std::string(16,'n'),key="manager-001",witness,plan,token;
 unsigned major=1,minor=23,wall=60000;std::uint32_t uid=256,hfid=361;
 struct Fields{bool empty()const{return !mock::unknown;}};struct Reflection{Fields GetUnknownFields(const Request&)const{return {};}};
 const Reflection *GetReflection()const{static Reflection r;return &r;}bool IsInitialized()const{return mock::initialized;}
 std::size_t ByteSize()const{return 128+bearer.size()+nonce.size()+key.size()+witness.size()+plan.size()+token.size();}
 const std::string &bearer_token()const{return bearer;}const std::string &client_nonce()const{return nonce;}
 unsigned protocol_major()const{return major;}unsigned protocol_minor()const{return minor;}unsigned max_wall_millis()const{return wall;}
#define MA_FIELD(type,name,member,bit) bool has_##name()const{return(mask&(bit))!=0;}type name()const{return member;}
 MA_FIELD(std::uint32_t,unit_id,uid,1) MA_FIELD(std::uint32_t,histfig_id,hfid,2)
 MA_FIELD(const std::string&,idempotency_key,key,4) MA_FIELD(const std::string&,expected_witness,witness,8)
 MA_FIELD(const std::string&,plan_digest,plan,16) MA_FIELD(const std::string&,prepare_token,token,32)
#undef MA_FIELD
};
struct Reply {
 bool accepted=false,replayed=false,unresolved=false;std::uint32_t failure=0,major=0,minor=0,retained=0;std::uint64_t generation=0;
 std::string nonce,df_version,dfhack_version,observation,effect;
 void Clear(){*this=Reply();}std::size_t ByteSize()const{return 128+observation.size()+effect.size();}
 void set_accepted(bool v){accepted=v;}void set_failure_code(unsigned v){failure=v;}void set_client_nonce(const std::string&v){nonce=v;}
 void set_protocol_major(unsigned v){major=v;}void set_protocol_minor(unsigned v){minor=v;}void set_bridge_generation(std::uint64_t v){generation=v;}
 void set_df_version(const std::string&v){df_version=v;}void set_dfhack_version(const std::string&v){dfhack_version=v;}
 void set_observation(const std::string&v){observation=v;}void set_effect_record(const std::string&v){effect=v;}
 void set_replayed(bool v){replayed=v;}void set_unresolved(bool v){unresolved=v;}void set_retained_records(unsigned v){retained=v;}
};
}
