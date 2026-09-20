#include "bridge/common/order_run.h"
#include <iostream>
#include <stdexcept>
#include <vector>
using namespace dfmcp_order_run;
namespace {
int checks = 0;
void check_impl(bool value, int line) { ++checks; if (!value) throw std::runtime_error("line " + std::to_string(line)); }
#define check(value) check_impl((value), __LINE__)
template<class F> void refused(unsigned code, F body) {
    try { body(); } catch (const clock::Failure &e) { check(code == e.code); return; }
    check(false);
}
struct Case {
    Engine engine;
    Capture value{{41, 0, 100, true, true, true}, {41, 7, "fort"}, 9, 10, true, 1, 10, 10, 0};
    Clock::time_point now{std::chrono::seconds(100)};
    std::string plan = std::string(32, 'p');
    int unpauses = 0, pauses = 0, reads = 0;
    bool fail_read = false, fail_pause = false, fail_after_unpause = false, fail_after_pause = false;
    auto reader() { return [this](std::uint32_t id) {
        ++reads; check(id == 9);
        if (fail_read) throw clock::Failure(5);
        return value;
    }; }
    auto writer() { return [this](const Identity &identity, bool paused) {
        require(value.identity == identity && value.clock.loaded, 4);
        if (paused) {
            ++pauses; if (fail_pause) throw clock::Failure(5);
            value.clock.paused = true; if (fail_after_pause) throw clock::Failure(5);
        } else {
            ++unpauses; value.clock.paused = false;
            if (fail_after_unpause) throw clock::Failure(5);
        }
    }; }
    const Record &prepare(Goal goal = {}, clock::Spec limits = {100, 1000}, std::string key = "run") {
        return engine.prepare(key, plan, limits, goal, engine.observe(9, reader()), now, reader());
    }
    const Record &commit(std::string key = "run") { return engine.commit(key, plan, now, reader(), writer()); }
    void step(std::uint64_t ticks = 1, int ms = 1) {
        value.clock.tick += ticks; now += std::chrono::milliseconds(ms);
        engine.service(now, reader(), writer());
    }
};
void predicates() {
    for (auto p : {Predicate::Approved, Predicate::Active, Predicate::RemainingAtMost}) {
        Case c; Goal g{p, p == Predicate::RemainingAtMost ? 5u : 0u, 1, 1};
        const auto &r = c.prepare(g); c.commit(); check(c.unpauses == 1 && c.engine.active());
        c.value.status = p == Predicate::Approved ? 1u : p == Predicate::Active ? 2u : 0u;
        c.value.left = p == Predicate::RemainingAtMost ? 5 : 10;
        c.step(); check(r.predicate_observed() && r.run->phase == clock::Phase::Stopped);
        check(r.run->pause_verified && r.sample && r.sample->clock.tick == 101 && !r.sample->clock.paused);
        check(r.run->observed_tick == 101 && c.pauses == 1 && !c.engine.active());
        const auto samples = r.stable_samples; const auto captured = *r.sample;
        c.value.status = 0; c.step(1); c.commit();
        check(c.unpauses == 1 && r.stable_samples == samples && *r.sample == captured);
    }
}
void temporal_accounting() {
    Case c; const auto &r = c.prepare({Predicate::Approved, 0, 2, 3}); c.commit(); c.value.status = 1;
    c.step(2); check(r.stable_samples == 0);
    c.step(1); check(r.stable_samples == 1 && c.engine.active());
    for (int i = 0; i < 4; ++i) c.step(0, 0);
    check(r.stable_samples == 1);
    c.value.status = 0; c.step(0, 0); check(r.stable_samples == 0);
    c.value.status = 1; c.step(3); check(r.stable_samples == 1);
    c.step(2); check(c.engine.active()); c.step(1); check(r.predicate_observed());
    // Independent discrete oracle over every eight-sample truth schedule.
    for (unsigned mask = 0; mask < 256; ++mask) {
        Case d; const auto &s = d.prepare({Predicate::Approved, 0, 3, 1}); d.commit();
        unsigned streak = 0; bool triggered = false;
        for (unsigned i = 0; i < 8; ++i) {
            const bool positive = (mask & (1u << i)) != 0;
            if (!triggered) { streak = positive ? streak + 1 : 0; triggered = streak == 3; }
            d.value.status = positive ? 1 : 0; d.step();
            check(s.predicate_observed() == triggered);
            check(s.run->pause_verified == triggered);
        }
        check(d.unpauses == 1);
    }
}
void anomaly_stops() {
    for (int mode = 0; mode < 6; ++mode) {
        Case c; const auto &r = c.prepare(); c.commit();
        Trigger expected = Trigger::None;
        if (mode == 0) { c.value.present = false; c.value.recipe = 0; c.value.total = c.value.left = 0; expected = Trigger::TargetAbsent; }
        if (mode == 1) { c.value.recipe = 2; expected = Trigger::TargetChanged; }
        if (mode == 2) { c.value.total = 11; expected = Trigger::TargetChanged; }
        if (mode == 3) { c.value.recipe = 0; expected = Trigger::TargetChanged; }
        if (mode == 4) { c.value.left = 8; c.step(); c.value.left = 9; expected = Trigger::CounterRegression; }
        if (mode == 5) { c.value.next_order = 11; c.step(); c.value.next_order = 10; expected = Trigger::HorizonRegression; }
        c.step(); check(r.trigger == expected && !r.predicate_observed());
        check(r.run->pause_verified && r.run->phase == clock::Phase::Stopped && c.pauses == 1);
    }
    // Disappearance at zero cannot be counted as successful production.
    Case c; const auto &r = c.prepare({Predicate::RemainingAtMost, 0, 2, 1}); c.commit();
    c.value.left = 0; c.step(); check(r.stable_samples == 1 && !r.predicate_observed());
    c.value.present = false; c.value.recipe = 0; c.value.total = 0; c.step();
    check(r.trigger == Trigger::TargetAbsent && !r.predicate_observed());
}
void source_and_preparation_fences() {
    for (int mode = 0; mode < 3; ++mode) {
        Case c; const auto &r = c.prepare(); c.commit();
        if (mode == 0) c.value.identity.folder = "other";
        if (mode == 1) ++c.value.identity.site;
        if (mode == 2) { ++c.value.identity.generation; ++c.value.clock.generation; }
        c.step(); check(r.run->phase == clock::Phase::SourceLost && !r.run->pause_verified);
        check(c.pauses == 0 && r.trigger == Trigger::SourceChanged && !c.engine.active());
    }
    for (int mode = 0; mode < 6; ++mode) {
        Case c; const auto &r = c.prepare();
        if (mode == 0) c.value.identity.folder = "other";
        if (mode == 1) c.value.status = 1;
        if (mode == 2) ++c.value.clock.tick;
        if (mode == 3) c.value.left = 9;
        if (mode == 4) c.now += clock::PREPARE_LIFETIME;
        if (mode == 5) c.fail_read = true;
        c.commit(); check(r.run->phase == clock::Phase::Refused && c.unpauses == 0);
    }
    Case c; const auto &a = c.prepare(); const auto &b = c.prepare({}, {100, 1000}, "next"); c.commit();
    refused(8, [&] { c.commit("next"); });
    c.value.status = 1; c.step(); check(a.predicate_observed());
    c.value.status = 0; c.commit("next"); check(b.run->phase == clock::Phase::Refused && c.unpauses == 1);
    refused(7, [&] { c.engine.query("run", std::string(32, 'q')); });
}
void limits_faults_and_cancellation() {
    for (int mode = 0; mode < 3; ++mode) {
        Case c; const auto &r = c.prepare(); c.commit(); c.value.status = 1;
        if (mode == 0) c.step(100, 1);
        if (mode == 1) c.step(1, 1000);
        if (mode == 2) { c.value.clock.paused = true; c.step(); }
        check(r.run->pause_verified && !r.predicate_observed());
        check(r.run->reason == (mode == 0 ? clock::Reason::TickLimit : mode == 1 ? clock::Reason::WallLimit : clock::Reason::ExternalPause));
    }
    Case c; const auto &r = c.prepare(); c.commit(); c.fail_pause = true; c.value.status = 1; c.step();
    check(r.predicate_observed() && r.run->phase == clock::Phase::Stopping && !r.run->pause_verified);
    const auto evidence = *r.sample;
    c.value.status = 0; c.step(); check(*r.sample == evidence && c.unpauses == 1 && c.engine.active());
    refused(8, [&] { c.prepare({}, {100, 1000}, "next"); });
    c.fail_pause = false; c.fail_after_pause = true; c.step(); check(r.run->pause_verified && *r.sample == evidence);
    Case d; const auto &s = d.prepare(); d.fail_after_unpause = true; d.commit();
    check(d.unpauses == 1 && s.run->pause_verified && !s.predicate_observed());
    d.commit(); check(d.unpauses == 1);
    Case e; const auto &t = e.prepare(); e.commit(); e.fail_read = true; e.step();
    check(e.value.clock.paused && t.run->phase == clock::Phase::Stopping && !t.run->pause_verified);
    e.fail_read = false; e.step(); check(t.run->pause_verified && !t.predicate_observed());
    Case f; const auto &u = f.prepare();
    f.engine.cancel("run", f.plan, f.now, f.reader(), f.writer()); f.commit();
    check(u.run->phase == clock::Phase::Refused && f.unpauses == 0);
    Case h; const auto &v = h.prepare(); h.commit(); h.fail_pause = true;
    check(!h.engine.shutdown(h.now, h.reader(), h.writer()));
    h.fail_pause = false; check(h.engine.shutdown(h.now, h.reader(), h.writer()) && v.run->pause_verified);
}
void validation_and_capacity() {
    for (Goal goal : {Goal{Predicate::Approved,1,1,1}, Goal{Predicate::Approved,0,0,1},
        Goal{Predicate::Approved,0,17,1}, Goal{Predicate::Approved,0,1,0},
        Goal{Predicate::Approved,0,16,100}, Goal{Predicate::RemainingAtMost,11,1,1},
        Goal{static_cast<Predicate>(0),0,1,1}}) {
        Case c; refused(3, [&] { c.prepare(goal); }); check(c.engine.size() == 0);
    }
    Case c; c.value.status = 1; refused(4, [&] { c.prepare(); });
    c.value.status = 0; c.value.recipe = 0; refused(4, [&] { c.prepare(); });
    c.value.recipe = 1;
    const auto before = c.engine.observe(9, c.reader());
    for (std::size_t i = 0; i < clock::MAX_RECORDS; ++i) c.prepare({}, {100, 1000}, "run-" + std::to_string(i));
    refused(5, [&] { c.prepare(); }); check(c.engine.size() == clock::MAX_RECORDS);
    const auto *r = c.engine.query("run-0", c.plan); check(r != nullptr);
    auto prepared = r->run->prepared; auto reads = c.reads;
    c.engine.prepare("run-0", c.plan, {100,1000}, {}, before, c.now + std::chrono::seconds(50), c.reader());
    check(c.reads == reads && r->run->prepared == prepared);
}
}
int main() {
    predicates(); temporal_accounting(); anomaly_stops(); source_and_preparation_fences();
    limits_faults_and_cancellation(); validation_and_capacity();
    std::cout << "order-run engine: " << checks << " assertions passed\n";
}
