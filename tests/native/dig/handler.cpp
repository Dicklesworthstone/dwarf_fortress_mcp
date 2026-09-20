#include "../../../bridge/dfhack-dig-v1_15/dfmcp_dig_v1_15.cpp"
#include <iostream>
#include <stdexcept>
static unsigned assertions=0;
#define CHECK(x) do{++assertions;if(!(x))throw std::runtime_error("CHECK failed: " #x);}while(false)
static color_ostream output;
static dg::Region target{15,15,2,2,2};
void reset() {
    engine=dg::Engine(7);Core::getInstance()=Core{};
    Version::value="53.test-r1";World::fort=true;World::paused=true;World::year=0;World::tick=12345;World::site=1;World::folder="region1";
    Maps::present.fill(true);Maps::calls=0;Maps::fail_at=0;Maps::on_read={};Maps::sx=64;Maps::sy=64;Maps::sz=8;
    for(auto &b:Maps::blocks){b=df::map_block{};for(unsigned x=0;x<16;++x)for(unsigned y=0;y<16;++y){
        b.tiletype[x][y]=df::tiletype::StoneWall;b.temperature_1[x][y]=10015;b.temperature_2[x][y]=10015;}}
    wire::Reply::fail_effect=false;
    CHECK(setenv("DFMCP_ALLOW_UNADMITTED_DIG_V1_15","1",1)==0);
    CHECK(setenv("DFMCP_DIG_ALLOW_DESIGNATE","1",1)==0);CHECK(setenv("DFMCP_DIG_TOKEN",std::string(32,'k').c_str(),1)==0);
}
wire::Request request(unsigned mask=0) {
    wire::Request r;r.set_bearer_token(std::string(32,'k'));r.set_client_nonce(std::string(16,'n'));r.set_protocol_major(1);r.set_protocol_minor(15);
    if(mask&1)r.set_x(target.x);
    if(mask&2)r.set_y(target.y);
    if(mask&4)r.set_z(target.z);
    if(mask&8)r.set_width(target.width);
    if(mask&16)r.set_height(target.height);
    if(mask&32)r.set_idempotency_key("mine-001");
    if(mask&64)r.set_expected_witness(std::string(32,'w'));
    if(mask&128)r.set_plan_digest(std::string(32,'p'));
    if(mask&256)r.set_prepare_token(std::string(16,'t'));
    return r;
}
using Handler=command_result(*)(color_ostream &,const wire::Request *,wire::Reply *);
wire::Reply invoke(Handler f,const wire::Request &r){wire::Reply out;CHECK(f(output,&r,&out)==CR_OK);return out;}
wire::Request prepare() {
    auto r=request(255);const auto before=engine.inspect(target,capture);
    r.set_expected_witness(before.witness());r.set_plan_digest(dg::plan_digest(before.witness()));
    const auto out=invoke(PrepareDig,r);CHECK(out.accepted() && out.effect_ && out.replayed_==false);return r;
}
wire::Request commit_request(const wire::Request &p) {
    auto r=request(416);r.set_plan_digest(p.plan_digest());r.set_prepare_token(dg::token_for(7,p.idempotency_key(),p.plan_digest()));return r;
}
void exact_capture_and_effect() {
    reset();auto h=invoke(Handshake,request());CHECK(h.accepted() && h.generation_==7 && h.minor_==15);
    for(unsigned width=1;width<=8;++width)for(unsigned height=1;height<=8;++height){
        reset();target={15,15,2,width,height};auto read=invoke(ReadDig,request(31));CHECK(read.accepted());
        const auto before=engine.inspect(target,capture);CHECK(read.observation_==before.encode());
        const auto p=prepare();const auto commit=commit_request(p);const auto out=invoke(CommitDig,commit);
        CHECK(out.accepted() && out.effect_);const auto *record=engine.query("mine-001",p.plan_digest());
        CHECK(record && record->state==dg::State::Designated && record->designated_tiles==width*height);
        CHECK(record->after_witness==before.expected_after().witness());
        CHECK(World::paused && World::tick==12345);
        bool exact=true;
        for(unsigned z=0;z<8;++z)for(unsigned y=0;y<64;++y)for(unsigned x=0;x<64;++x){
            auto *b=Maps::getTileBlock(x,y,z);const auto &d=b->designation[x&15][y&15];
            auto masked=d;masked.bits.dig=0;
            if(d.bits.dig!=(target.target(x,y,z)?1u:0u) || masked.whole || b->occupancy[x&15][y&15].whole
                || b->flags.bits.designated!=target.touched_block(x,y,z)
                || b->tiletype[x&15][y&15]!=df::tiletype::StoneWall)exact=false;
        }
        CHECK(exact);const auto calls=Maps::calls;
        CHECK(invoke(CommitDig,commit).effect_==out.effect_);CHECK(Maps::calls==calls);
        auto query=request(160);query.set_plan_digest(p.plan_digest());CHECK(invoke(QueryDig,query).effect_==out.effect_);CHECK(Maps::calls==calls);
    }
    target={15,15,2,2,2};
}
void authentication_shapes_and_revocation() {
    reset();for(auto entry:std::vector<std::pair<Handler,unsigned>>{{Handshake,0},{ReadDig,31},{PrepareDig,255},{CommitDig,416},{QueryDig,160}}){
        for(unsigned mask=0;mask<512;++mask)if(mask!=entry.second){
            const auto calls=Maps::calls;auto out=invoke(entry.first,request(mask));
            CHECK(!out.accepted() && out.failure_code()==3);CHECK(!out.observation_ && !out.effect_ && Maps::calls==calls);
        }
    }
    for(unsigned bad=0;bad<8;++bad){reset();auto r=request(31);
        switch(bad){case 0:r.unknown=true;break;case 1:r.required=7;break;case 2:r.set_protocol_minor(14);break;
            case 3:r.set_client_nonce("short");break;case 4:r.set_bearer_token(std::string(32,'z'));break;
            case 5:r.set_bearer_token(std::string(257,'k'));break;case 6:unsetenv("DFMCP_ALLOW_UNADMITTED_DIG_V1_15");break;
            default:Core::getInstance().vinfo.reset();}
        auto out=invoke(ReadDig,r);CHECK(!out.accepted() && !out.observation_ && !out.effect_ && Maps::calls==0);
    }
    reset();const auto p=prepare();unsetenv("DFMCP_DIG_ALLOW_DESIGNATE");
    CHECK(invoke(CommitDig,commit_request(p)).failure_code()==1);
    CHECK(invoke(PrepareDig,p).failure_code()==1);
    CHECK(invoke(ReadDig,request(31)).accepted());
    auto q=request(160);q.set_plan_digest(p.plan_digest());CHECK(invoke(QueryDig,q).accepted());
    CHECK(engine.query("mine-001",p.plan_digest())->state==dg::State::Prepared);
}
void hidden_hazards_and_staleness() {
    reset();auto *b=Maps::getTileBlock(15,15,2);b->designation[15][15].bits.hidden=true;
    const auto a=invoke(ReadDig,request(31));CHECK(a.accepted());
    b->tiletype[15][15]=static_cast<df::tiletype>(-999);b->occupancy[15][15].whole=UINT32_MAX;b->temperature_1[15][15]=UINT16_MAX;
    const auto second=invoke(ReadDig,request(31));CHECK(second.accepted() && second.observation_==a.observation_);
    for(unsigned kind=0;kind<12;++kind){reset();b=Maps::getTileBlock(15,15,2);auto &d=b->designation[15][15].bits;auto &o=b->occupancy[15][15].bits;
        switch(kind){case 0:d.hidden=1;break;case 1:d.water_table=1;break;case 2:d.feature_local=1;break;
            case 3:d.flow_size=1;break;case 4:o.heavy_aquifer=1;break;case 5:o.dig_auto=1;break;
            case 6:o.unit=1;break;case 7:d.smooth=1;break;case 8:b->tiletype[15][15]=df::tiletype::Construction;break;
            case 9:b->tiletype[15][15]=df::tiletype::SmoothWall;break;case 10:b->temperature_1[15][15]=10075;break;
            default:b->tiletype[15][15]=df::tiletype::StoneFloor;}
        const auto before=engine.inspect(target,capture);CHECK(!before.eligible());
        auto p=request(255);p.set_expected_witness(before.witness());p.set_plan_digest(dg::plan_digest(before.witness()));
        CHECK(invoke(PrepareDig,p).failure_code()==4);
    }
    for(unsigned kind=0;kind<4;++kind){reset();const auto p=prepare();
        if(kind==0)++World::tick;else if(kind==1)Maps::getTileBlock(14,14,1)->occupancy[14][14].bits.other=1;
        else if(kind==2)plugin_onstatechange(output,SC_UNPAUSED);else World::folder="another";
        auto out=invoke(CommitDig,commit_request(p));CHECK(out.accepted());CHECK(engine.query("mine-001",p.plan_digest())->state==dg::State::Refused);
        CHECK(Maps::getTileBlock(15,15,2)->designation[15][15].bits.dig==0);
    }
}
void reply_loss_preflight_and_readback() {
    reset();auto p=prepare();wire::Reply::fail_effect=true;auto out=invoke(CommitDig,commit_request(p));
    CHECK(!out.accepted() && out.failure_code()==5);const auto *r=engine.query("mine-001",p.plan_digest());CHECK(r && r->state==dg::State::Designated);
    auto q=request(160);q.set_plan_digest(p.plan_digest());CHECK(invoke(QueryDig,q).effect_==r->encode());
    reset();p=prepare();Maps::calls=0;Maps::fail_at=51;out=invoke(CommitDig,commit_request(p));
    CHECK(out.accepted() && engine.query("mine-001",p.plan_digest())->state==dg::State::Unknown);
    CHECK(Maps::getTileBlock(15,15,2)->designation[15][15].bits.dig==0);
    reset();p=prepare();Maps::calls=0;Maps::fail_at=53;out=invoke(CommitDig,commit_request(p));
    CHECK(out.accepted() && engine.query("mine-001",p.plan_digest())->state==dg::State::Unknown);
    CHECK(Maps::getTileBlock(15,15,2)->designation[15][15].bits.dig==1);
    const auto calls=Maps::calls;CHECK(invoke(CommitDig,commit_request(p)).effect_==out.effect_);CHECK(Maps::calls==calls);
    for(auto event:{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_MAP_LOADED,SC_MAP_UNLOADED}){
        reset();p=prepare();CHECK(plugin_onstatechange(output,event)==CR_OK);CHECK(engine.generation()==8);
        q=request(160);q.set_plan_digest(p.plan_digest());out=invoke(QueryDig,q);CHECK(out.accepted() && !out.effect_);
        CHECK(invoke(CommitDig,commit_request(p)).failure_code()==7);
    }
    reset();std::vector<PluginCommand> commands;CHECK(plugin_init(output,commands)==CR_OK && commands.empty());
    std::unique_ptr<RPCService> rpc(plugin_rpcconnect(output));
    CHECK((rpc->methods==std::vector<std::pair<std::string,unsigned>>{{"Handshake",0},{"ReadDig",0},{"PrepareDig",0},{"CommitDig",0},{"QueryDig",0}}));
    CHECK(plugin_shutdown(output)==CR_OK);
}
int main(){try{exact_capture_and_effect();authentication_shapes_and_revocation();hidden_hazards_and_staleness();reply_loss_preflight_and_readback();
    std::cout<<"{\"groups\":4,\"assertions\":"<<assertions<<"}\n";return 0;
}catch(const std::exception &e){std::cerr<<e.what()<<'\n';return 1;}}
