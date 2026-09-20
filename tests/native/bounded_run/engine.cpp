#include "bridge/common/bounded_run.h"
#include <cassert>
#include <iostream>
#include <stdexcept>

using namespace dfmcp_bounded_run;
namespace {
int checks = 0;
void check(bool value) { ++checks; if (!value) throw std::runtime_error("assertion failed"); }
template<class F> void refused(unsigned code, F f) {
    try { f(); } catch (const Failure &e) { check(e.code == code); return; }
    check(false);
}
struct World {
    Snapshot value{41, 0, 100, true, true, true};
    int unpauses = 0, pauses = 0, read_failures = 0;
    bool fail_before_unpause = false, fail_after_unpause = false;
    bool fail_pause = false, fail_after_pause = false, ignore_pause = false;
    Snapshot read() {
        if (read_failures > 0) { --read_failures; throw std::runtime_error("read"); }
        return value;
    }
    void write(std::uint64_t generation, bool paused) {
        check(value.loaded && value.generation == generation);
        if (paused) {
            ++pauses;
            if (fail_pause) throw std::runtime_error("pause");
            if (!ignore_pause) value.paused = true;
            if (fail_after_pause) throw std::runtime_error("after pause");
        } else {
            ++unpauses;
            if (fail_before_unpause) throw std::runtime_error("before unpause");
            value.paused = false;
            if (fail_after_unpause) throw std::runtime_error("after unpause");
        }
    }
};
struct Case {
    Engine engine; World world;
    Clock::time_point now{std::chrono::seconds(100)};
    std::string plan = std::string(32, 'p');
    auto reader() { return [this] { return world.read(); }; }
    auto writer() { return [this](std::uint64_t g, bool p) { world.write(g, p); }; }
    const Record &prepare(const std::string &key = "run", Spec spec = {10, 1000}) {
        auto expected = engine.observe(reader());
        return engine.prepare(key, plan, spec, expected, now, reader());
    }
    const Record &commit(const std::string &key = "run") {
        return engine.commit(key, plan, now, reader(), writer());
    }
    void step(std::uint64_t ticks = 0, int ms = 0) {
        world.value.tick += ticks; now += std::chrono::milliseconds(ms);
        engine.service(now, reader(), writer());
    }
};
void limits_and_retries() {
    Case c; const auto &r = c.prepare();
    check(r.phase == Phase::Prepared && !r.unpause_attempted && !c.engine.active());
    check(c.commit().phase == Phase::Running && c.world.unpauses == 1);
    c.commit(); check(c.world.unpauses == 1);
    c.step(9, 10); check(c.engine.active() && !c.world.value.paused);
    c.step(1, 10);
    check(r.phase == Phase::Stopped && r.reason == Reason::TickLimit && r.pause_verified);
    check(r.tick_known && r.observed_tick == 110 && !c.engine.active());
    c.commit(); c.step(100); check(c.world.unpauses == 1 && c.world.pauses == 1);
    check(c.engine.query("missing", c.plan) == nullptr);
    refused(7, [&] { c.engine.query("run", std::string(32, 'q')); });
}
void wall_without_ticks_and_overshoot() {
    Case c; const auto &r = c.prepare(); c.commit();
    c.step(0, 999); check(c.engine.active()); c.step(0, 1);
    check(r.reason == Reason::WallLimit && r.pause_verified && r.observed_tick == 100);
    Case d; const auto &s = d.prepare(); d.commit(); d.step(25, 1500);
    check(s.pause_verified && s.observed_tick == 125 && s.reason == Reason::WallLimit);
    // Actual overshoot remains visible; stopping is not an exact-tick claim.
    check(s.observed_tick - s.before.tick > s.spec.game_ticks);
}
void cancellation_and_shutdown() {
    Case c; const auto &r = c.prepare();
    c.engine.cancel("run", c.plan, c.now, c.reader(), c.writer());
    c.commit(); check(r.phase == Phase::Refused && r.reason == Reason::Cancelled && c.world.unpauses == 0);
    c.prepare("next"); c.commit("next");
    const auto &next = c.engine.cancel("next", c.plan, c.now, c.reader(), c.writer());
    check(next.pause_verified && next.reason == Reason::Cancelled);
    c.engine.cancel("next", c.plan, c.now, c.reader(), c.writer()); check(c.world.pauses == 1);
    Case d; const auto &s = d.prepare(); d.commit(); d.world.fail_pause = true;
    check(!d.engine.shutdown(d.now, d.reader(), d.writer()) && d.engine.active());
    check(s.phase == Phase::Stopping && !s.pause_verified);
    d.world.fail_pause = false;
    check(d.engine.shutdown(d.now, d.reader(), d.writer()) && s.reason == Reason::Shutdown);
}
void authority_and_staleness() {
    Case c; c.prepare("a"); const auto &b = c.prepare("b"); c.commit("a");
    refused(8, [&] { c.commit("b"); }); refused(8, [&] { c.prepare("new"); });
    c.step(10); c.commit("b"); check(b.phase == Phase::Refused && c.world.unpauses == 1);
    for (int mode = 0; mode < 5; ++mode) {
        Case d; const auto &r = d.prepare();
        if (mode == 0) d.now += PREPARE_LIFETIME;
        if (mode == 1) d.now -= std::chrono::milliseconds(1);
        if (mode == 2) ++d.world.value.tick;
        if (mode == 3) d.world.value.paused = false;
        if (mode == 4) d.world.read_failures = 1;
        d.commit(); check(r.phase == Phase::Refused && !r.unpause_attempted && d.world.unpauses == 0);
    }
    Case d; const auto &r = d.prepare(); const auto prepared = r.prepared;
    d.now += std::chrono::seconds(59); d.prepare(); check(r.prepared == prepared);
    d.now += std::chrono::seconds(1); d.commit(); check(r.phase == Phase::Refused);
}
void ambiguous_effects_and_retrying_only_safety_pauses() {
    for (int mode = 0; mode < 2; ++mode) {
        Case c; const auto &r = c.prepare();
        c.world.fail_before_unpause = mode == 0; c.world.fail_after_unpause = mode == 1;
        c.commit(); check(r.phase == Phase::Stopped && r.reason == Reason::NativeFailure && r.pause_verified);
        c.commit(); check(c.world.unpauses == 1);
    }
    Case c; const auto &r = c.prepare(); c.commit(); c.world.ignore_pause = true; c.step(10);
    check(r.phase == Phase::Stopping && c.engine.active() && !r.pause_verified);
    c.commit(); c.step(); check(c.world.unpauses == 1 && c.world.pauses == 2);
    refused(8, [&] { c.prepare("other"); });
    c.world.ignore_pause = false; c.world.fail_after_pause = true; c.step();
    check(r.phase == Phase::Stopped && r.pause_verified && !c.engine.active());
    Case d; const auto &s = d.prepare(); d.commit(); d.world.read_failures = 2; d.step();
    check(s.phase == Phase::Stopping && !s.pause_verified && d.world.value.paused);
    d.step(); check(s.phase == Phase::Stopped && s.reason == Reason::NativeFailure);
}
void source_loss_and_clock_failures() {
    Case c; const auto &r = c.prepare(); c.commit(); ++c.world.value.generation; c.step();
    check(r.phase == Phase::SourceLost && !r.pause_verified && c.world.pauses == 0);
    check(!c.engine.active()); c.commit(); check(c.world.unpauses == 1);
    Case d; const auto &s = d.prepare(); d.engine.source_changed(); d.commit();
    check(s.phase == Phase::Refused && s.reason == Reason::SourceChanged && d.world.unpauses == 0);
    Case e; const auto &t = e.prepare(); e.commit(); e.engine.source_changed();
    check(t.phase == Phase::SourceLost && !t.pause_verified && e.world.pauses == 0);
    for (int mode = 0; mode < 3; ++mode) {
        Case f; const auto &u = f.prepare(); f.commit();
        if (mode == 0) f.world.value.tick = 99;
        if (mode == 1) f.now -= std::chrono::milliseconds(1);
        if (mode == 2) f.world.value.clock_valid = false;
        f.step(); check(u.phase == Phase::Stopped && u.pause_verified);
        check(u.reason == (mode == 2 ? Reason::NativeFailure : Reason::ClockRegression));
        if (mode == 2) check(!u.tick_known);
    }
    Case f; const auto &u = f.prepare(); f.commit(); f.world.value.paused = true; f.step();
    check(u.reason == Reason::ExternalPause && u.pause_verified && f.world.pauses == 0);
}
void validation_and_capacity() {
    Case c;
    for (Spec spec : {Spec{0, 1}, Spec{1201, 1}, Spec{1, 0}, Spec{1, 60001}})
        refused(3, [&] { c.prepare("bad", spec); });
    refused(3, [&] { c.prepare("bad key"); });
    c.world.value.paused = false; refused(4, [&] { c.prepare(); });
    c.world.value.paused = true; c.world.value.tick = UINT64_MAX;
    refused(5, [&] { c.prepare(); });
    c.world.value.tick = 100; c.now = Clock::time_point::max();
    refused(5, [&] { c.prepare(); });
    Case d;
    for (std::size_t i = 0; i < MAX_RECORDS; ++i) d.prepare("run-" + std::to_string(i));
    check(d.engine.size() == MAX_RECORDS);
    refused(5, [&] { d.prepare("overflow"); });
    check(d.engine.query("run-0", d.plan) != nullptr);
    // A duplicate at capacity is still retrievable; it never renews or redispatches.
    d.prepare("run-0"); check(d.world.unpauses == 0);
}
void deterministic_run_matrix() {
    for (std::uint32_t ticks : {1u, 2u, 10u, 1200u}) {
        for (std::uint32_t wall : {1u, 10u, 1000u, 60000u}) {
            for (int stop = 0; stop < 4; ++stop) {
                Case c; const auto &r = c.prepare("run", {ticks, wall}); c.commit();
                if (stop == 0) c.step(ticks, 0);
                if (stop == 1) c.step(0, static_cast<int>(wall));
                if (stop == 2) c.engine.cancel("run", c.plan, c.now, c.reader(), c.writer());
                if (stop == 3) c.engine.shutdown(c.now, c.reader(), c.writer());
                check(r.phase == Phase::Stopped && r.pause_verified && !c.engine.active());
                c.commit(); check(c.world.unpauses == 1);
            }
        }
    }
}
} // namespace
int main() {
    limits_and_retries(); wall_without_ticks_and_overshoot(); cancellation_and_shutdown();
    authority_and_staleness(); ambiguous_effects_and_retrying_only_safety_pauses();
    source_loss_and_clock_failures(); validation_and_capacity(); deterministic_run_matrix();
    std::cout << "bounded-run engine: " << checks << " assertions passed\n";
}
