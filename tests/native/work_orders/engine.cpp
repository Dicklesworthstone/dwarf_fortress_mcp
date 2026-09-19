#include "work_order_creation.h"
#include <functional>
#include <iostream>
#include <stdexcept>

namespace w = dfmcp_work_orders;
static unsigned assertions = 0, groups = 0;
#define CHECK(x) do { ++assertions; if (!(x)) throw std::runtime_error(#x); } while (false)
template<class F> void rejects(F f, unsigned code = 0) {
    bool rejected = false;
    try { f(); } catch (const w::Failure &error) { rejected = true; CHECK(code == 0 || error.code == code); }
    CHECK(rejected);
}
std::string hex(const std::string &value) {
    const char *digits = "0123456789abcdef"; std::string out;
    for (unsigned char c : value) { out.push_back(digits[c >> 4]); out.push_back(digits[c & 15]); } return out;
}
struct Model {
    w::Observation observation;
    unsigned reads = 0, writes = 0, verifications = 0;
    std::string observed_config;
    Model() { observation.tick = 12345; observation.next_order = 10; observation.site = 1;
        observation.paused = true; observation.folder = "region1"; observation.ids = {2, 6}; }
    w::Observation read() { ++reads; return observation; }
    void write(std::uint32_t id, w::Spec spec) {
        ++writes; CHECK(id == observation.next_order); observation.ids.push_back(id); ++observation.next_order;
        observed_config = w::configuration(id, spec);
    }
    std::string verify(std::uint32_t, w::Spec) { ++verifications; return observed_config; }
};
struct Fixture {
    w::Engine engine{7}; Model model;
    w::Spec spec{w::Recipe::WoodenBed, 5};
    w::Clock::time_point now{std::chrono::seconds(1000)};
    w::Record prepare(const std::string &key = "order-001") {
        const auto obs = engine.inspect([&] { return model.read(); });
        return *engine.prepare(key, spec, obs.witness(), w::plan_digest(spec, obs.witness()), now,
            [&] { return model.read(); }).first;
    }
    const w::Record &commit(const w::Record &r, w::Clock::time_point at) {
        return engine.commit(r.key, r.plan, r.token, at, [&] { return model.read(); },
            [&](auto id, auto s) { model.write(id, s); }, [&](auto id, auto s) { return model.verify(id, s); });
    }
    const w::Record &commit(const w::Record &r) { return commit(r, now); }
};
void bounds() {
    ++groups;
    for (unsigned recipe = 0; recipe <= 255; ++recipe) {
        const w::Spec s{static_cast<w::Recipe>(recipe), 1};
        if (recipe >= 1 && recipe <= 4) CHECK(s.encode().size() == 5); else rejects([&] { s.validate(); });
    }
    for (auto amount : {0u, 101u, 32768u, UINT32_MAX}) rejects([&] { w::Spec{w::Recipe::WoodenBed, amount}.validate(); });
    w::Spec{w::Recipe::WoodenChair, 100}.validate();
    for (const auto &key : {std::string(), std::string(129, 'x'), std::string("has space"), std::string("a/b"), std::string("x\0y", 3)})
        rejects([&] { w::valid_key(key); });
    w::valid_key("A-z_0.9"); w::valid_key(std::string(128, 'a'));
    CHECK(w::utf8("region\xc3\xa9", 512));
    for (const auto &bad : {std::string(), std::string("a\0b", 3), std::string("\xc0\xaf"), std::string("\xed\xa0\x80"),
        std::string("\xf4\x90\x80\x80"), std::string("\xc3"), std::string(513, 'a')}) CHECK(!w::utf8(bad, 512));
    Fixture f; auto o = f.engine.inspect([&] { return f.model.read(); });
    for (auto ids : {std::vector<std::uint32_t>{2, 2}, {6, 2}, {2, 10}, {UINT32_MAX}}) {
        auto bad = o; bad.ids = ids; rejects([&] { (void)bad.encode(); }, 5);
    }
    auto bad = o; bad.generation = 0; rejects([&] { (void)bad.encode(); });
    bad = o; bad.sequence = UINT64_MAX; rejects([&] { (void)bad.encode(); });
    bad = o; bad.tick = std::uint64_t{UINT32_MAX} * 403200 + 403200; rejects([&] { (void)bad.encode(); });
    bad = o; bad.site = UINT32_MAX; rejects([&] { (void)bad.encode(); });
    bad = o; bad.next_order = UINT32_MAX; rejects([&] { (void)bad.encode(); });
    bad = o; bad.ids.resize(w::MAX_ORDERS + 1); rejects([&] { (void)bad.encode(); });
    CHECK(o.encode().size() == 62);
}
void success_and_replay() {
    ++groups;
    for (auto recipe : {w::Recipe::WoodenBed, w::Recipe::WoodenDoor, w::Recipe::WoodenTable, w::Recipe::WoodenChair}) {
        Fixture f; f.spec.recipe = recipe; auto r = f.prepare();
        CHECK(r.state == w::State::Prepared && !r.terminal() && f.model.writes == 0);
        const auto reads = f.model.reads;
        auto again = f.engine.prepare(r.key, f.spec, r.witness, r.plan, f.now + std::chrono::hours(1), [&] { return f.model.read(); });
        CHECK(again.second && again.first->encode() == r.encode() && f.model.reads == reads);
        auto done = f.commit(r); CHECK(done.state == w::State::Created && done.terminal() && done.after_known);
        CHECK(f.model.writes == 1 && f.model.verifications == 1 && !f.engine.unresolved());
        CHECK(done.after_witness == r.before.expected_after().witness());
        CHECK(done.configuration_witness == dfmcp_snapshot::sha256(w::configuration(10, f.spec)));
        CHECK(done.receipt == done.proof()); CHECK(f.commit(r).encode() == done.encode()); CHECK(f.model.writes == 1);
        CHECK(f.engine.query(r.key, r.plan)->encode() == done.encode()); CHECK(f.engine.query("absent", r.plan) == nullptr);
        rejects([&] { (void)f.engine.query(r.key, std::string(32, 'x')); }, 7);
        auto wrong = r; wrong.token[0] ^= 1; rejects([&] { (void)f.commit(wrong); }, 7);
        CHECK(f.model.writes == 1);
        if (recipe == w::Recipe::WoodenBed) {
            std::cout << "observation=" << hex(r.before.encode()) << '\n';
            std::cout << "prepared=" << hex(r.encode()) << '\n';
            std::cout << "created=" << hex(done.encode()) << '\n';
        }
    }
}
void stale_and_expired() {
    ++groups;
    const std::vector<std::function<void(Model &)>> changes{
        [](auto &m) { ++m.observation.tick; }, [](auto &m) { ++m.observation.next_order; },
        [](auto &m) { m.observation.ids[0] = 1; }, [](auto &m) { m.observation.ids.pop_back(); },
        [](auto &m) { m.observation.paused = false; }, [](auto &m) { m.observation.folder = "other"; },
        [](auto &m) { ++m.observation.site; }};
    for (const auto &change : changes) {
        Fixture f; auto r = f.prepare(); change(f.model); auto denied = f.commit(r);
        CHECK(denied.state == w::State::Refused && !denied.after_known && f.model.writes == 0);
        CHECK(denied.receipt == denied.proof()); CHECK(f.commit(r).encode() == denied.encode());
    }
    for (const auto seconds : {-1, 60, 61, 100000}) {
        Fixture f; auto r = f.prepare(); auto denied = f.commit(r, f.now + std::chrono::seconds(seconds));
        CHECK(denied.state == w::State::Refused && f.model.writes == 0);
        if (seconds == 60) std::cout << "refused=" << hex(denied.encode()) << '\n';
    }
    Fixture f; auto r = f.prepare(); CHECK(f.commit(r, f.now + std::chrono::seconds(60) - std::chrono::nanoseconds(1)).state == w::State::Created);
    Fixture paused; auto p = paused.prepare(); paused.engine.interrupt();
    CHECK(paused.commit(p).state == w::State::Refused && paused.model.writes == 0);
    Fixture failed; auto z = failed.prepare();
    const auto &denied = failed.engine.commit(z.key, z.plan, z.token, failed.now,
        []() -> w::Observation { throw std::runtime_error("read failed"); },
        [&](auto id, auto s) { failed.model.write(id, s); }, [&](auto id, auto s) { return failed.model.verify(id, s); });
    CHECK(denied.state == w::State::Refused && failed.model.writes == 0);
}
void competing_and_unknown() {
    ++groups;
    Fixture f; auto a = f.prepare("a"), b = f.prepare("b");
    CHECK(f.commit(a).state == w::State::Created); CHECK(f.commit(b).state == w::State::Refused); CHECK(f.model.writes == 1);
    for (bool insert_first : {false, true}) {
        Fixture u; auto x = u.prepare(), y = u.prepare("other");
        const auto &unknown = u.engine.commit(x.key, x.plan, x.token, u.now, [&] { return u.model.read(); },
            [&](auto id, auto s) { if (insert_first) u.model.write(id, s); throw std::runtime_error("writer fault"); },
            [&](auto id, auto s) { return u.model.verify(id, s); });
        CHECK(unknown.state == w::State::Unknown && !unknown.after_known && u.engine.unresolved());
        CHECK(unknown.receipt == std::string(32, '\0'));
        const auto writes = u.model.writes; CHECK(u.commit(x).encode() == unknown.encode()); CHECK(u.model.writes == writes);
        rejects([&] { (void)u.commit(y); }, 8); rejects([&] { (void)u.prepare("new"); }, 8);
        CHECK(u.engine.query(x.key, x.plan)->state == w::State::Unknown);
        if (insert_first) std::cout << "unknown=" << hex(unknown.encode()) << '\n';
        const auto generation = u.engine.generation(); u.engine.reset();
        CHECK(u.engine.generation() == generation + 1 && !u.engine.unresolved()); CHECK(u.engine.query(x.key, x.plan) == nullptr);
        CHECK(u.model.writes == writes); // Absence after reset is not undo or proof of non-creation.
    }
}
void exact_readback() {
    ++groups;
    const std::vector<std::function<void(Model &)>> corruptions{
        [](auto &m) { m.observed_config[0] ^= 1; }, [](auto &m) { m.observed_config.back() ^= 1; },
        [](auto &m) { ++m.observation.tick; }, [](auto &m) { m.observation.ids[0] = 1; },
        [](auto &m) { m.observation.ids.pop_back(); }, [](auto &m) { ++m.observation.next_order; },
        [](auto &m) { m.observation.paused = false; }, [](auto &m) { ++m.observation.site; },
        [](auto &m) { m.observation.folder = "other"; }};
    for (const auto &change : corruptions) {
        Fixture f; auto r = f.prepare();
        const auto &out = f.engine.commit(r.key, r.plan, r.token, f.now, [&] { return f.model.read(); },
            [&](auto id, auto s) { f.model.write(id, s); change(f.model); }, [&](auto id, auto s) { return f.model.verify(id, s); });
        CHECK(out.state == w::State::Unknown && !out.after_known && out.receipt == std::string(32, '\0'));
        CHECK(f.model.writes == 1 && f.engine.unresolved());
    }
    Fixture f; auto r = f.prepare();
    const auto &out = f.engine.commit(r.key, r.plan, r.token, f.now, [&] { return f.model.read(); },
        [](auto, auto) {}, [](auto, auto) -> std::string { throw std::runtime_error("readback failed"); });
    CHECK(out.state == w::State::Unknown); // No-op insertion is NOT a verified creation.
}
void capacity_and_horizons() {
    ++groups;
    Fixture f;
    for (std::size_t i = 0; i < w::MAX_RECORDS; ++i) (void)f.prepare("key-" + std::to_string(i));
    CHECK(f.engine.size() == w::MAX_RECORDS); rejects([&] { (void)f.prepare("overflow"); }, 5);
    auto old = f.prepare("key-0"); CHECK(old.state == w::State::Prepared);
    Fixture full; full.model.observation.ids.clear();
    for (std::uint32_t i = 0; i < w::MAX_ORDERS; ++i) full.model.observation.ids.push_back(i);
    full.model.observation.next_order = w::MAX_ORDERS;
    CHECK(full.engine.inspect([&] { return full.model.read(); }).encode().size() <= w::MAX_OBSERVATION_BYTES);
    rejects([&] { (void)full.prepare(); }, 4);
    Fixture final; final.model.observation.next_order = INT32_MAX - 1;
    auto p = final.prepare(); CHECK(final.commit(p).state == w::State::Created);
    CHECK(final.model.observation.next_order == INT32_MAX); rejects([&] { (void)final.prepare("new"); }, 4);
    w::Engine zero(0), max(UINT64_MAX), last(UINT64_MAX - 1);
    CHECK(!zero.available() && !max.available()); last.reset(); CHECK(!last.available()); last.reset(); CHECK(last.generation() == UINT64_MAX);
    Fixture edge; auto observation = edge.engine.inspect([&] { return edge.model.read(); });
    observation.sequence = UINT64_MAX - 1; CHECK(!observation.eligible()); rejects([&] { (void)observation.expected_after(); }, 4);
    Fixture time; time.now = w::Clock::time_point::max() - std::chrono::seconds(1);
    auto t = time.prepare(); CHECK(time.commit(t, w::Clock::time_point::max()).state == w::State::Created);
}
int main() {
    try { bounds(); success_and_replay(); stale_and_expired(); competing_and_unknown(); exact_readback(); capacity_and_horizons();
        std::cout << "engine_groups=" << groups << "\nengine_assertions=" << assertions << '\n'; return 0; }
    catch (const std::exception &error) { std::cerr << error.what() << '\n'; return 1; }
}
