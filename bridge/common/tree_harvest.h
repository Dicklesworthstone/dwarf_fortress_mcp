#pragma once
#include "workshop_construction.h"
// A coordinate/source-bound predicate, never a stable plant identity.
// This model shares transaction state, not workshop or mining semantics.
namespace dfmcp_tree_harvest {
using dfmcp_workshop::require;
using dfmcp_workshop::u16;
using dfmcp_workshop::u32;
using dfmcp_workshop::i32;
using dfmcp_workshop::u64;
using dfmcp_workshop::flag;
using dfmcp_workshop::field;
using dfmcp_workshop::hash;
using dfmcp_workshop::key_valid;
using dfmcp_workshop::Coord;
using dfmcp_workshop::Worker;
constexpr std::size_t MAX_CELLS = 2048, MAX_TOOLS = 8;
constexpr std::size_t MAX_CAPTURE_BYTES = 128 * 1024, MAX_RECORD_BYTES = 384 * 1024;
constexpr std::uint32_t MAX_PLANTS = 65536, MAX_ITEMS = 1048576, MAX_JOBS = 65536, MAX_UNITS = 8192;
struct Selection {
    Coord root;
    std::string encode() const { return root.encode(); }
};
struct Cell {
    Coord pos;
    std::uint32_t tiletype = 0, designation_other = 0, occupancy = 0, block_other = 0;
    std::uint16_t temperature1 = 0, temperature2 = 0;
    unsigned char presence = 0, dig = 0, liquid = 0;
    bool designated = false;
    std::string encode() const {
        require(presence <= 2 && dig <= 7 && liquid <= 7, 5);
        std::string out = pos.encode(); out.push_back(static_cast<char>(presence));
        if (presence != 2) {
            require(!tiletype && !designation_other && !occupancy && !block_other && !temperature1
                && !temperature2 && !dig && !liquid && !designated, 5);
            return out;
        }
        for (auto n : {tiletype, designation_other, occupancy, block_other}) u32(out, n);
        u16(out, temperature1); u16(out, temperature2);
        out.push_back(static_cast<char>(dig)); out.push_back(static_cast<char>(liquid)); flag(out, designated);
        return out;
    }
};
struct Tool {
    std::uint32_t id = 0, flags = 0, flags2 = 0, holder = 0;
    std::int32_t role = -1;
    Coord pos;
    bool axe = false, visible = false, reciprocal = false, reachable = false;
    std::string encode() const {
        require(id < INT32_MAX && holder < INT32_MAX && role >= -1 && role <= 2, 5);
        std::string out; for (auto n : {id, flags, flags2, holder}) u32(out, n);
        i32(out, role); out += pos.encode();
        for (auto v : {axe, visible, reciprocal, reachable}) flag(out, v);
        return out;
    }
    bool offer() const {
        // An offered axe is never an equipment, ownership or scheduling guarantee.
        constexpr auto computed = dfmcp_workshop::COMPUTED_ITEM_FLAGS;
        constexpr auto foreign = std::uint32_t{1} << 14;
        return axe && visible && reciprocal && reachable && !flags2
            && !(flags & ~(computed | foreign)) && role == 0;
    }
};
struct Capture {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, next_item = 0;
    std::array<std::uint32_t, 3> dimensions{};
    std::string folder;
    Selection selection;
    unsigned char root_presence = 0;
    std::int32_t species = -1;
    std::uint32_t width = 0, height = 0, body_height = 0;
    Coord designation;
    bool paused = false, mature = false, alive = false, complete = false, existing_job = false;
    std::vector<std::uint16_t> body;
    std::vector<Cell> cells;
    std::vector<Worker> workers;
    std::vector<Tool> tools;
    std::string encode() const {
        require(generation && generation < UINT64_MAX && sequence < UINT64_MAX
            && tick <= dfmcp_workshop::MAX_TICK && site <= INT32_MAX && next_item <= INT32_MAX
            && dfmcp_workshop::utf8(folder,512) && root_presence <= 2, 5);
        for (auto d : dimensions) require(d && d <= 32768, 5);
        require(selection.root.x < dimensions[0] && selection.root.y < dimensions[1]
            && selection.root.z < dimensions[2], 5);
        require(body.size() <= MAX_CELLS && cells.size() <= MAX_CELLS && workers.size() <= 32 && tools.size() <= MAX_TOOLS, 5);
        require((root_presence == 2 && mature) || (body.empty() && !width && !height && !body_height), 5);
        std::string out = "DFMTC021";
        for (auto n : {generation,sequence,tick}) u64(out,n);
        u32(out,site); for (auto n : dimensions) u32(out,n); u32(out,next_item); field(out,folder);
        out += selection.encode(); out.push_back(static_cast<char>(root_presence)); i32(out,species);
        for (auto n : {width,height,body_height}) u32(out,n);
        out += designation.encode();
        for (bool v : {paused,mature,alive,complete,existing_job}) flag(out,v);
        u16(out,static_cast<std::uint16_t>(body.size())); for (auto n : body) u16(out,n);
        u16(out,static_cast<std::uint16_t>(cells.size())); for (const auto &c : cells) out += c.encode();
        out.push_back(static_cast<char>(workers.size())); for (const auto &w : workers) out += w.encode();
        out.push_back(static_cast<char>(tools.size())); for (const auto &t : tools) out += t.encode();
        require(out.size() <= MAX_CAPTURE_BYTES, 5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    bool eligible() const {
        (void)encode();
        if (!paused || root_presence != 2 || !mature || !alive || !complete || existing_job
            || species < 0 || body.empty() || cells.empty() || sequence >= UINT64_MAX-1) return false;
        bool target = false, actor = false, tool = false;
        for (const auto &c : cells) {
            if (c.presence != 2 || c.liquid || c.temperature1 > 10080 || c.temperature2 > 10080) return false;
            if (c.pos == designation) {
                if (target || c.dig || c.occupancy) return false;
                target = true;
            }
        }
        for (const auto &w : workers) actor = actor || w.eligible();
        for (const auto &t : tools) tool = tool || t.offer();
        return target && actor && tool;
    }
    Capture expected_after() const {
        require(eligible(),4); auto out=*this; ++out.sequence;
        for (auto &c : out.cells) {
            if (c.pos.z == designation.z && c.pos.x/16 == designation.x/16 && c.pos.y/16 == designation.y/16)
                c.designated=true;
            if (c.pos == designation) c.dig=1;
        }
        return out;
    }
};
struct Insertion {
    Coord root, designation;
    std::int32_t species = -1;
    bool marked = false;
    bool matches(const Capture &before) const {
        return root == before.selection.root && designation == before.designation && species == before.species && marked;
    }
    std::string encode() const {
        std::string out="DFMTI021"; out+=root.encode(); out+=designation.encode(); i32(out,species); flag(out,marked); return out;
    }
};
enum class Phase : unsigned char { Prepared=0, Indeterminate=1, Placed=2, Refused=3, Cancelled=4 };
enum class Reason : unsigned char { None=0, Stale=1, Expired=2, SourceChanged=3, Cancelled=4, NativeFailure=5 };
inline std::string plan_digest(const Selection &s,const std::string &w) {
    require(w.size()==32); return hash("dfmcp-tree-harvest-plan/1",s.encode()+w);
}
inline std::string token_for(const std::string &k,const std::string &p) {
    key_valid(k); require(p.size()==32); std::string out; field(out,k); out+=p; return hash("dfmcp-tree-harvest-token/1",out).substr(0,16);
}
struct Progress {};
struct Record {
    std::string key,plan,token;
    Capture before;
    std::optional<Capture> after;
    std::optional<Insertion> insertion;
    Phase phase=Phase::Prepared;
    Reason reason=Reason::None;
    std::uint64_t created_ms=0;
    std::string encode() const {
        std::string out="DFMTR021"; field(out,key); out+=plan; out+=token; const auto b=before.encode(); u32(out,static_cast<std::uint32_t>(b.size())); out+=b;
        out.push_back(static_cast<char>(phase)); out.push_back(static_cast<char>(reason));
        flag(out,after.has_value()); if(after) { const auto a=after->encode(); u32(out,static_cast<std::uint32_t>(a.size())); out+=a; }
        flag(out,insertion.has_value()); if(insertion) out+=insertion->encode();
        out+=hash("dfmcp-tree-harvest-receipt/1",out);
        require(out.size()<=MAX_RECORD_BYTES,5); return out;
    }
};
struct Model {
    using Capture=dfmcp_tree_harvest::Capture; using Selection=dfmcp_tree_harvest::Selection;
    using Progress=dfmcp_tree_harvest::Progress; using Record=dfmcp_tree_harvest::Record;
    using Phase=dfmcp_tree_harvest::Phase; using Reason=dfmcp_tree_harvest::Reason;
    static constexpr std::size_t MAX_RECORDS=64;
    static constexpr std::uint64_t PREPARE_MS=60000;
    static void require(bool v,std::uint32_t c=3) { dfmcp_tree_harvest::require(v,c); }
    static void key_valid(const std::string &k) { dfmcp_tree_harvest::key_valid(k); }
    static std::string plan_digest(const Selection &s,const std::string &w) { return dfmcp_tree_harvest::plan_digest(s,w); }
    static std::string token_for(const std::string &k,const std::string &p) { return dfmcp_tree_harvest::token_for(k,p); }
};
using Engine=dfmcp_one_attempt::Engine<Model>;
} // namespace dfmcp_tree_harvest
