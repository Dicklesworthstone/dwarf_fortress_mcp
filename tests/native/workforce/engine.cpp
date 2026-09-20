#include "bridge/common/workforce_control.h"
#include <iostream>
#include <stdexcept>
using namespace dfmcp_workforce;
namespace {
unsigned checks = 0;
void check(bool v) { ++checks; if (!v) throw std::runtime_error("workforce assertion failed"); }
template<class F> void refused(unsigned code, F f) {
    try { f(); } catch (const Failure &e) { check(e.code == code); return; } check(false);
}
struct Case {
    Engine engine{42}; Capture state;
    int writes = 0, recomputes = 0;
    int failure = 0;
    Clock::time_point now{std::chrono::seconds(100)};
    Case() {
        state.tick = 100; state.site = 7; state.folder = "region1";
        state.paused = state.automatic = true; state.labor_keys = {"MINE", "CARPENTER", "HAUL"};
        Detail d; d.name = "Miners"; d.flags = 2; d.selected_only = true;
        d.labors = std::string("\1\0\0", 3); d.members = {5}; state.details.push_back(d);
        d.name = "Carpenters"; d.labors = std::string("\0\1\0", 3); d.members = {2}; state.details.push_back(d);
        state.units = {{2, 102, true, std::string("\0\1\1", 3)}, {5, 105, true, std::string("\1\0\1", 3)}};
    }
    auto read() { return [this](const std::vector<std::uint32_t> &ids) {
        if (failure == 1) throw Failure(5);
        check(ids == state.ids()); return state;
    }; }
    auto write() { return [this](const Capture &before, Spec spec,
        const std::vector<std::uint32_t> &members, const std::vector<std::uint32_t> &changed) {
        ++writes; if (failure == 2) throw Failure(5);
        check(before.folder == state.folder && before.site == state.site);
        state.details[spec.detail].members = members;
        if (failure == 3) throw Failure(5);
        for (auto &u : state.units) if (std::binary_search(changed.begin(), changed.end(), u.id)) {
            ++recomputes;
            if (failure != 4) u.labors[0] = spec.assigned ? 1 : 0;
        }
        if (failure == 5) state.details[1].members.clear();
        if (failure == 6) state.units[1].historical_id++;
        if (failure == 7) state.units[1].labors[2] = 0; // Unchanged unit must not be recomputed.
        if (failure == 8) state.automatic = false;
        if (failure == 9) state.folder = "replacement";
    }; }
    Capture observed() { return engine.observe(state.ids(), read()); }
    const Record &prepare(const std::string &key = "assign", Spec spec = {0, true}) {
        const auto capture = observed(); const auto witness = capture.witness();
        return engine.prepare(key, spec, capture.ids(), witness, plan_digest(spec, witness), now, read());
    }
    const Record &commit(const Record &r) { return engine.commit(r.key, r.plan, r.token, now, read(), write()); }
};
void lifecycle() {
    Case c; const auto &r = c.prepare(); const auto plan = r.plan;
    check(r.phase == Phase::Prepared && c.writes == 0);
    c.prepare(); check(c.engine.size() == 1);
    c.commit(r); check(r.phase == Phase::Applied && !c.engine.unresolved());
    check(c.writes == 1 && c.recomputes == 1 && c.state.details[0].members == std::vector<std::uint32_t>({2,5}));
    check(r.after_units.size() == 2 && r.after_units[0].labors[0] == 1);
    check(r.after_witness == c.observed().witness());
    c.commit(r); c.engine.cancel(r.key,r.plan,r.token); check(c.writes == 1);
    check(c.engine.query(r.key, plan) == &r);
    refused(4,[&] { c.prepare("noop"); });
    const auto &remove = c.prepare("remove", {0,false}); c.commit(remove);
    check(remove.phase == Phase::Applied && c.state.details[0].members.empty());
    check(c.recomputes == 3);
    refused(7,[&] { c.engine.query(r.key,std::string(32,'x')); });
    check(c.engine.query("absent",plan) == nullptr);
}
void partial_and_readback() {
    for (int fault = 2; fault <= 9; ++fault) {
        Case c; const auto &r = c.prepare(); c.failure = fault; c.commit(r);
        check(r.phase == Phase::Unknown && c.engine.unresolved());
        check(c.writes == 1 && r.after_units.empty() && r.after_witness == std::string(32,'\0'));
        c.failure = 0; c.commit(r); check(c.writes == 1);
        c.engine.cancel(r.key,r.plan,r.token); check(r.phase == Phase::Unknown);
        refused(8,[&] { c.prepare("other"); });
    }
}
void staleness_and_cancellation() {
    for (int fault = 0; fault < 10; ++fault) {
        Case c; const auto &r = c.prepare();
        if (fault == 0) ++c.state.tick;
        if (fault == 1) c.state.paused = false;
        if (fault == 2) c.state.automatic = false;
        if (fault == 3) c.state.details[0].name = "Renamed";
        if (fault == 4) c.state.details[1].members.push_back(9);
        if (fault == 5) ++c.state.units[0].historical_id;
        if (fault == 6) c.state.units[0].labors[2] = 0;
        if (fault == 7) c.now += LIFETIME;
        if (fault == 8) c.now -= std::chrono::milliseconds(1);
        if (fault == 9) c.failure = 1;
        c.commit(r); check(r.phase == Phase::Refused && c.writes == 0);
        c.failure = 0; c.commit(r); check(c.writes == 0);
    }
    Case c; const auto &r = c.prepare(); c.engine.cancel(r.key,r.plan,r.token); c.commit(r);
    check(r.phase == Phase::Cancelled && c.writes == 0);
    Case d; const auto &a = d.prepare(); const auto &b = d.prepare("second"); d.commit(a); d.commit(b);
    check(b.phase == Phase::Refused && d.writes == 1);
    const auto plan = a.plan; d.engine.reset(); check(d.engine.query("assign",plan) == nullptr);
    check(d.engine.generation() == 43);
}
void bounds() {
    Case c;
    for (const std::string &key : {std::string(), std::string("wrong key"), std::string(129,'a')}) refused(3,[&] { c.prepare(key); });
    c.state.units[0].eligible = false; refused(4,[&] { c.prepare(); }); c.state.units[0].eligible = true;
    c.state.details[0].selected_only = false; refused(4,[&] { c.prepare(); }); c.state.details[0].selected_only = true;
    c.state.details[0].members = {5,5}; refused(3,[&] { c.prepare(); }); c.state.details[0].members = {5};
    c.state.units[0].labors[0] = 2; refused(3,[&] { c.prepare(); }); c.state.units[0].labors[0] = 0;
    c.state.folder = std::string("\xc0\x80",2); refused(5,[&] { c.prepare(); });
    Case d; for (std::size_t i = 0; i < MAX_RECORDS; ++i) d.prepare("r"+std::to_string(i));
    refused(5,[&] { d.prepare("overflow"); }); d.prepare("r0"); check(d.engine.size() == MAX_RECORDS);
    Case e; e.now = Clock::time_point::max(); refused(5,[&] { e.prepare(); });
}
void membership_matrix() {
    for (unsigned mask = 0; mask < 256; ++mask) for (bool desired : {false,true}) {
        Case c; c.state.units.clear(); c.state.details.resize(1); c.state.details[0].members.clear();
        for (unsigned id = 0; id < 8; ++id) {
            const bool old = (mask & (1u << id)) != 0;
            if (old) c.state.details[0].members.push_back(id);
            c.state.units.push_back({id,100+id,true,std::string({static_cast<char>(old), '\0', '\1'})});
        }
        if ((!mask && !desired) || (mask == 255 && desired)) { refused(4,[&] { c.prepare("x",{0,desired}); }); continue; }
        const auto &r = c.prepare("x",{0,desired}); c.commit(r); check(r.phase == Phase::Applied);
        for (const auto &u : c.state.units) check(bool(u.labors[0]) == desired);
        check(c.state.details[0].members.size() == (desired ? 8 : 0));
        c.commit(r); check(c.writes == 1);
    }
}
void vector() {
    Case c; const auto &r = c.prepare();
    auto hex = [](const std::string &s) { const char *h = "0123456789abcdef"; std::string out;
        for (unsigned char b : s) { out += h[b>>4]; out += h[b&15]; } return out; };
    std::cout << "capture=" << hex(r.before.encode()) << '\n';
    std::cout << "prepared=" << hex(r.encode()) << '\n'; c.commit(r);
    std::cout << "applied=" << hex(r.encode()) << '\n';
}
}
int main() { lifecycle(); partial_and_readback(); staleness_and_cancellation(); bounds(); membership_matrix(); vector();
    std::cout << "workforce engine: " << checks << " assertions passed\n"; }
