#include "../build_placement.h"
#include <functional>
#include <iostream>
#include <stdexcept>
#include <type_traits>
using namespace dfmcp_build;
namespace {
std::size_t checks = 0, groups = 0;
#define CHECK(x) do { ++checks; if (!(x)) throw std::runtime_error(std::string("check failed: ") + #x + " at " + std::to_string(__LINE__)); } while (false)
template<class F> void refused(F f, std::uint32_t code = 0) {
    bool caught = false;
    try { f(); } catch (const Failure &e) { caught = !code || e.code == code; }
    CHECK(caught);
}
std::string hex(const std::string &v) {
    static const char *digits = "0123456789abcdef";
    std::string out;
    for (unsigned char c : v) { out += digits[c >> 4]; out += digits[c & 15]; }
    return out;
}
Capture initial(Kind kind = Kind::Bed) {
    Capture c; c.generation = 41; c.sequence = 0; c.tick = 806500; c.site = 2;
    c.next_building = 70; c.next_job = 90; c.building_count = 4;
    c.dimensions = {64, 64, 8}; c.folder = "region1"; c.paused = c.free_tile = c.supported = true;
    c.selection = Selection{kind, 42, Coord{15, 15, 2}};
    for (auto &t : c.tiles) { t.presence = Presence::Visible; t.tiletype = 37; t.shape = 3; }
    c.item.presence = Presence::Visible; c.item.pos = {10, 11, 2}; c.item.kind = kind;
    c.item.native_type = 100 + static_cast<unsigned>(kind); c.item.subtype = -1;
    c.item.material = 419; c.item.material_index = -1; c.item.quality = 2; c.item.on_ground = true;
    c.item.ground = c.tiles[0]; c.item.ground.occupancy_other = 8;
    return c;
}
Insertion inserted(const Capture &c) {
    Insertion p; p.building = c.next_building; p.job = c.next_job; p.item = c.selection.item;
    p.kind = c.selection.kind; p.pos = c.selection.target; p.material = c.item.material;
    p.material_index = c.item.material_index; p.max_stage = 1;
    p.linked = p.construct_job = p.exact_item_link = true; return p;
}
struct Game {
    Capture value = initial();
    Insertion proof = inserted(value);
    std::size_t reads = 0, writes = 0, verifies = 0;
    bool fail_read = false, no_write = false, throw_after = false, fail_verify = false;
    std::function<void()> during_write, during_verify;
    Capture read(const Selection &) { ++reads; if (fail_read) throw Failure(5); return value; }
    void write(const Capture &c) {
        ++writes;
        if (during_write) during_write();
        if (no_write) throw Failure(5);
        value = c.expected_after();
        if (throw_after) throw Failure(5);
    }
    Insertion verify(const Capture &) { ++verifies; if (during_verify) during_verify(); if (fail_verify) throw Failure(5); return proof; }
};
struct Fixture {
    Engine engine{41}; Game game;
    std::string key = "golden", plan, token;
    Capture observe() { return engine.inspect(game.value.selection, [&](const Selection &s) { return game.read(s); }); }
    const Record &prepare(std::uint64_t now = 100) {
        const auto c = observe(); plan = plan_digest(c.selection, c.witness()); token = token_for(key, plan);
        return engine.prepare(key, c.selection, c.witness(), plan, now, [&](const Selection &s) { return game.read(s); });
    }
    const Record &commit(std::uint64_t now = 101) {
        try {
            return engine.commit(key, plan, token, now, [&](const Selection &s) { return game.read(s); },
                [&](const Capture &c) { game.write(c); }, [&](const Capture &c) { return game.verify(c); });
        } catch (const Failure &) {
            throw std::runtime_error("check failed: authenticated commit/replay must return retained evidence");
        }
    }
};
void vectors() {
    Fixture f; const auto before = f.observe(); const auto prepared = f.prepare().encode();
    const auto placed = f.commit().encode();
    Fixture bad; bad.prepare(); bad.game.no_write = true; const auto unknown = bad.commit().encode();
    Fixture stale; stale.prepare(); const auto expired = stale.commit(60100).encode();
    Fixture cancelled; cancelled.prepare(); const auto retired = cancelled.engine.cancel(cancelled.key, cancelled.plan, cancelled.token).encode();
    std::cout << "{";
    bool first = true;
    for (const auto &v : std::vector<std::pair<std::string, std::string>>{
        {"capture", before.encode()}, {"plan", f.plan}, {"token", f.token},
        {"prepared", prepared}, {"placed", placed}, {"indeterminate", unknown}, {"expired", expired}, {"cancelled", retired}}) {
        if (!first) std::cout << ',';
        first = false; std::cout << '"' << v.first << "\":\"" << hex(v.second) << '"';
    }
    std::cout << "}\n";
}
void suite() {
    static_assert(std::is_nothrow_move_assignable<Record>::value, "publication must not throw after verification");
    ++groups; // All admitted furniture families perform one witnessed insertion.
    for (Kind k : {Kind::Bed, Kind::Chair, Kind::Table}) {
        Fixture f; f.game.value = initial(k); f.game.proof = inserted(f.game.value);
        const auto p = f.prepare(); CHECK(p.phase == Phase::Prepared); CHECK(f.game.writes == 0);
        const auto r = f.commit(); CHECK(r.phase == Phase::Placed); CHECK(r.insertion->kind == k);
        CHECK(r.insertion->stage == 0); CHECK(r.insertion->item == 42); CHECK(r.after->next_job == 91);
        CHECK(f.game.writes == 1 && f.game.verifies == 1 && !f.engine.unresolved()); CHECK(r.encode().size() <= MAX_RECORD_BYTES);
    }
    ++groups; // Complete target shape/liquid/designation truth table.
    for (unsigned shape = 0; shape <= 8; ++shape) for (unsigned liquid = 0; liquid <= 7; ++liquid)
        for (unsigned dig = 0; dig <= 7; ++dig) {
            auto c = initial(); c.tiles[4].shape = static_cast<unsigned char>(shape);
            c.tiles[4].liquid = static_cast<unsigned char>(liquid); c.tiles[4].dig = static_cast<unsigned char>(dig);
            CHECK(c.eligible() == (shape == 3 && liquid == 0 && dig == 0));
        }
    ++groups; // Unknown or wet context must not disappear from placement decisions.
    for (std::size_t i = 0; i < 9; ++i) {
        for (Presence p : {Presence::Missing, Presence::Hidden}) {
            auto c = initial(); c.tiles[i] = Tile{}; c.tiles[i].presence = p; CHECK(!c.eligible());
            c.tiles[i].tiletype = 1; refused([&] { c.encode(); }, 5);
        }
        auto c = initial(); c.tiles[i].liquid = 1; CHECK(!c.eligible());
    }
    ++groups; // Strict exact-item eligibility, not an item-count check.
    std::vector<std::function<void(Capture &)>> blocked{
        [](auto &c) { c.paused = false; }, [](auto &c) { c.free_tile = false; }, [](auto &c) { c.supported = false; },
        [](auto &c) { c.tiles[4].occupied = true; }, [](auto &c) { c.tiles[4].building = 4; },
        [](auto &c) { c.tiles[4].occupancy_other = 8; }, [](auto &c) { c.item.kind = Kind::Other; },
        [](auto &c) { c.item.kind = Kind::Chair; }, [](auto &c) { c.item.on_ground = false; },
        [](auto &c) { c.item.in_job = true; }, [](auto &c) { c.item.other_flags = 1; },
        [](auto &c) { c.item.other_refs = 1; }, [](auto &c) { c.item.jobs = {9}; },
        [](auto &c) { c.item.wear = 1; }, [](auto &c) { c.item.material = -1; },
        [](auto &c) { c.item.ground.liquid = 1; }, [](auto &c) { c.item.ground.shape = 2; },
        [](auto &c) { c.item.ground.occupied = true; }, [](auto &c) { c.next_building = INT32_MAX; },
        [](auto &c) { c.next_job = INT32_MAX; }, [](auto &c) { c.building_count = MAX_BUILDINGS; },
        [](auto &c) { for (auto n : {1, 3, 5, 7}) c.tiles[n].shape = 2; },
    };
    for (const auto &change : blocked) { auto c = initial(); change(c); CHECK(!c.eligible()); }
    ++groups; // Hidden item data has no accidental attribute backing.
    for (Presence p : {Presence::Missing, Presence::Hidden}) {
        auto c = initial(); c.item = Item{}; c.item.presence = p;
        CHECK(!c.eligible()); CHECK(c.item.encode().size() == 1);
        c.item.pos.x = 1; refused([&] { c.encode(); }, 5);
    }
    ++groups; // Canonical source and input bounds.
    std::vector<std::function<void(Capture &)>> malformed{
        [](auto &c) { c.folder.clear(); }, [](auto &c) { c.folder = std::string(513, 'x'); },
        [](auto &c) { c.folder = std::string("x\0y", 3); }, [](auto &c) { c.folder = std::string("\xed\xa0\x80", 3); },
        [](auto &c) { c.generation = 0; }, [](auto &c) { c.generation = UINT64_MAX; },
        [](auto &c) { c.sequence = UINT64_MAX; }, [](auto &c) { c.tick = MAX_TICK + 1; },
        [](auto &c) { c.dimensions[0] = 0; }, [](auto &c) { c.dimensions[2] = 32769; },
        [](auto &c) { c.selection.target.x = 0; }, [](auto &c) { c.selection.target.y = 63; },
        [](auto &c) { c.selection.target.z = 8; }, [](auto &c) { c.selection.kind = Kind::Other; },
        [](auto &c) { c.selection.item = INT32_MAX; }, [](auto &c) { c.item.pos.x = 64; },
        [](auto &c) { c.item.jobs = {2, 2}; }, [](auto &c) { c.item.jobs.resize(9); },
        [](auto &c) { c.item.other_refs = 4097; }, [](auto &c) { c.tiles[0].liquid = 8; },
        [](auto &c) { c.tiles[0].shape = 9; }, [](auto &c) { c.tiles[0].presence = static_cast<Presence>(3); },
    };
    for (const auto &change : malformed) { auto c = initial(); change(c); refused([&] { c.encode(); }); }
    auto unicode = initial(); unicode.folder = "fort-\xc3\xa9-\xf0\x9f\x8f\xb0"; CHECK(unicode.eligible());
    ++groups; // Every relevant precondition is reread, including non-blocking changes.
    std::vector<std::function<void(Capture &)>> changed{
        [](auto &c) { ++c.tick; }, [](auto &c) { ++c.site; }, [](auto &c) { c.folder += "x"; },
        [](auto &c) { ++c.dimensions[0]; }, [](auto &c) { ++c.dimensions[1]; }, [](auto &c) { ++c.dimensions[2]; },
        [](auto &c) { ++c.next_building; }, [](auto &c) { ++c.next_job; }, [](auto &c) { ++c.building_count; },
        [](auto &c) { ++c.selection.item; }, [](auto &c) { ++c.selection.target.x; },
        [](auto &c) { ++c.selection.target.y; }, [](auto &c) { ++c.selection.target.z; },
        [](auto &c) { ++c.item.pos.x; }, [](auto &c) { ++c.item.pos.y; }, [](auto &c) { ++c.item.pos.z; },
        [](auto &c) { ++c.item.native_type; }, [](auto &c) { ++c.item.subtype; },
        [](auto &c) { ++c.item.material; }, [](auto &c) { ++c.item.material_index; },
        [](auto &c) { ++c.item.quality; }, [](auto &c) { ++c.item.ground.tiletype; },
        [](auto &c) { ++c.item.ground.occupancy_other; },
    };
    for (std::size_t i = 0; i < 9; ++i) changed.push_back([i](auto &c) { ++c.tiles[i].tiletype; });
    changed.insert(changed.end(), blocked.begin(), blocked.end());
    for (const auto &change : changed) {
        Fixture f; f.prepare(); change(f.game.value); const auto r = f.commit();
        CHECK(r.phase == Phase::Refused && r.reason == Reason::Stale); CHECK(f.game.writes == 0);
    }
    ++groups; // Absolute TTL; replay never extends it and backward time refuses.
    for (auto time : {std::uint64_t{99}, std::uint64_t{60100}, UINT64_MAX}) {
        Fixture f; const auto p = f.prepare();
        const auto &replay = f.engine.prepare(f.key, p.before.selection, p.before.witness(), f.plan, 60099,
            [&](const Selection &) -> Capture { throw std::runtime_error("replay read"); });
        CHECK(replay.created_ms == 100); CHECK(f.commit(time).phase == Phase::Refused); CHECK(f.game.writes == 0);
    }
    { Fixture f; f.prepare(); CHECK(f.commit(60099).phase == Phase::Placed); }
    ++groups; // Native scope/sequence changes invalidate without deleting history.
    { Fixture f; f.prepare(); f.engine.interrupt(); CHECK(f.commit().phase == Phase::Refused); CHECK(f.game.writes == 0); }
    { Fixture f; f.prepare(); f.engine.change_source(); CHECK(f.commit().reason == Reason::SourceChanged); CHECK(f.engine.size() == 1); }
    ++groups; // Token/key/plan cannot redirect an existing record.
    {
        Fixture f; f.prepare();
        refused([&] { f.engine.cancel(f.key, f.plan, std::string(16, 'x')); }, 7);
        refused([&] { f.engine.query(f.key, std::string(32, 'x')); }, 7);
        CHECK(!f.engine.query("absent", f.plan));
        auto altered = f.game.value.selection; altered.item = 43;
        refused([&] { f.engine.prepare(f.key, altered, f.game.value.witness(), f.plan, 100,
            [&](const Selection &s) { return f.game.read(s); }); }, 7);
        CHECK(f.game.writes == 0);
        for (const auto &key : std::vector<std::string>{"", "../key", "with space", std::string(129, 'x')}) refused([&] { key_valid(key); });
    }
    ++groups; // Pre-dispatch read failure proves no invocation; post-dispatch does not.
    { Fixture f; f.prepare(); f.game.fail_read = true; CHECK(f.commit().phase == Phase::Refused); CHECK(f.game.writes == 0); }
    { Fixture f; f.prepare(); f.game.during_write = [&] { f.game.fail_read = true; }; CHECK(f.commit().phase == Phase::Indeterminate); CHECK(f.game.writes == 1); }
    ++groups; // All indeterminate outcomes fence new keys and already prepared work.
    for (int fault = 0; fault < 4; ++fault) {
        Fixture f; f.prepare(); const auto old = *f.engine.query(f.key, f.plan);
        const auto second = f.engine.prepare("second", old.before.selection, old.before.witness(), f.plan, 100,
            [&](const Selection &s) { return f.game.read(s); });
        if (fault == 0) f.game.no_write = true;
        if (fault == 1) f.game.fail_verify = true;
        if (fault == 2) f.game.proof.exact_item_link = false;
        if (fault == 3) f.game.during_verify = [&] { ++f.game.value.item.quality; };
        CHECK(f.commit().phase == Phase::Indeterminate); CHECK(f.engine.unresolved());
        const auto bytes = f.engine.query(f.key, f.plan)->encode(); CHECK(f.commit().encode() == bytes); CHECK(f.game.writes == 1);
        f.game.value = initial(); // Even an unchanged, eligible map cannot prove nonapplication.
        const auto c = f.observe(); const auto p = plan_digest(c.selection, c.witness());
        refused([&] { f.engine.prepare("bypass", c.selection, c.witness(), p, 100,
            [&](const Selection &s) { return f.game.read(s); }); }, 8);
        refused([&] { f.engine.commit("second", second.plan, second.token, 100,
            [&](const Selection &s) { return f.game.read(s); }, [&](const Capture &v) { f.game.write(v); },
            [&](const Capture &v) { return f.game.verify(v); }); }, 8);
        CHECK(f.engine.cancel(f.key, f.plan, f.token).encode() == bytes); CHECK(f.engine.unresolved());
    }
    ++groups; // Throwing after all links were made may be resolved only by exact immediate readback.
    { Fixture f; f.prepare(); f.game.throw_after = true; CHECK(f.commit().phase == Phase::Placed); CHECK(f.game.writes == 1); }
    ++groups; // Exact registry, job, item and configuration proof is required.
    std::vector<std::function<void(Insertion &)>> bad_proofs{
        [](auto &p) { ++p.building; }, [](auto &p) { ++p.job; }, [](auto &p) { ++p.item; },
        [](auto &p) { p.kind = Kind::Chair; }, [](auto &p) { ++p.pos.x; }, [](auto &p) { ++p.pos.y; },
        [](auto &p) { ++p.pos.z; }, [](auto &p) { ++p.material; }, [](auto &p) { ++p.material_index; },
        [](auto &p) { p.stage = 1; }, [](auto &p) { p.max_stage = 0; }, [](auto &p) { p.max_stage = 33; },
        [](auto &p) { p.linked = false; }, [](auto &p) { p.construct_job = false; },
        [](auto &p) { p.exact_item_link = false; }, [](auto &p) { p.suspended = true; },
    };
    for (const auto &change : bad_proofs) {
        Fixture f; f.prepare(); change(f.game.proof); CHECK(f.commit().phase == Phase::Indeterminate); CHECK(f.engine.unresolved());
    }
    ++groups; // Full after-capture comparison, including the final verification read.
    for (const auto &change : changed) {
        auto probe = initial().expected_after(); const auto prior = probe.encode(); change(probe);
        if (probe.encode() == prior) continue;
        Fixture f; f.prepare(); f.game.during_verify = [&] { change(f.game.value); };
        CHECK(f.commit().phase == Phase::Indeterminate);
    }
    ++groups; // Source changes during a callback do not label replacement state as original-source evidence.
    { Fixture f; f.prepare(); f.game.during_write = [&] { f.engine.change_source(); }; CHECK(f.commit().phase == Phase::Indeterminate); }
    { Fixture f; f.prepare(); f.game.during_verify = [&] { f.engine.change_source(); }; CHECK(f.commit().phase == Phase::Indeterminate); }
    { Fixture f; refused([&] { f.engine.inspect(f.game.value.selection, [&](const Selection &s) { f.engine.interrupt(); return f.game.read(s); }); }, 6); }
    ++groups; // Retire only preparation; never confuse cancellation with deconstruction.
    {
        Fixture f; f.prepare(); const auto c = f.engine.cancel(f.key, f.plan, f.token).encode();
        CHECK(f.commit().encode() == c); CHECK(f.game.writes == 0); CHECK(!f.engine.unresolved());
        f.engine.change_source(); CHECK(f.engine.query(f.key, f.plan)->encode() == c);
    }
    ++groups; // Successful terminal replay does not invoke any callback or rewrite its evidence.
    {
        Fixture f; f.prepare(); const auto bytes = f.commit().encode();
        const auto reads = f.game.reads; f.game.fail_read = f.game.fail_verify = f.game.no_write = true;
        f.engine.change_source();
        CHECK(f.commit(UINT64_MAX).encode() == bytes); CHECK(f.game.reads == reads && f.game.writes == 1);
        CHECK(f.engine.cancel(f.key, f.plan, f.token).encode() == bytes);
    }
    ++groups; // Retention is bounded, terminal records are not evicted for new work.
    {
        Fixture f; const auto c = f.observe(); const auto p = plan_digest(c.selection, c.witness());
        for (std::size_t i = 0; i < MAX_RECORDS; ++i) {
            const auto key = "key-" + std::to_string(i);
            const auto r = f.engine.prepare(key, c.selection, c.witness(), p, 100, [&](const Selection &s) { return f.game.read(s); });
            f.engine.cancel(key, p, r.token);
        }
        CHECK(f.engine.size() == MAX_RECORDS);
        const auto reads = f.game.reads;
        refused([&] { f.engine.prepare("over-capacity", c.selection, c.witness(), p, 100, [&](const Selection &s) { return f.game.read(s); }); }, 5);
        CHECK(f.game.reads == reads); CHECK(f.engine.query("key-0", p)->phase == Phase::Cancelled);
    }
    ++groups; // The dispatch state and guard are published before the writer can reenter.
    {
        Fixture f; const auto before = f.prepare().before;
        f.game.during_write = [&] {
            CHECK(f.engine.query(f.key, f.plan)->phase == Phase::Indeterminate); CHECK(f.engine.unresolved());
            refused([&] { f.engine.prepare("reentrant", before.selection, before.witness(), f.plan, 100,
                [&](const Selection &s) { return f.game.read(s); }); }, 8);
            refused([&] { f.engine.cancel(f.key, f.plan, f.token); }, 8);
        };
        CHECK(f.commit().phase == Phase::Placed); CHECK(f.game.writes == 1);
    }
    ++groups; // Boundary counters use checked preconditions, including the last representable native ID.
    {
        Fixture f; f.game.value.next_building = INT32_MAX - 1; f.game.value.next_job = INT32_MAX - 1;
        f.game.value.building_count = MAX_BUILDINGS - 1; f.game.proof = inserted(f.game.value);
        f.prepare(); const auto r = f.commit(); CHECK(r.phase == Phase::Placed);
        CHECK(r.after->next_building == INT32_MAX && r.after->building_count == MAX_BUILDINGS); CHECK(!r.after->eligible());
        Engine exhausted(UINT64_MAX); refused([&] { exhausted.inspect(initial().selection, [&](const Selection &) { return initial(); }); }, 5);
    }
    ++groups; // Record encoding itself rejects internally inconsistent claims.
    {
        Fixture f; auto p = f.prepare(); p.phase = Phase::Placed; refused([&] { p.encode(); }, 5);
        p = f.commit(); p.after->item.jobs[0] += 1; refused([&] { p.encode(); }, 5);
        p = *f.engine.query(f.key, f.plan); p.insertion->exact_item_link = false; refused([&] { p.encode(); }, 5);
        p = *f.engine.query(f.key, f.plan); p.reason = Reason::Expired; refused([&] { p.encode(); }, 5);
    }
}
} // namespace
int main(int argc, char **argv) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--vectors") { vectors(); return 0; }
        if (argc != 1) return 2;
        suite(); std::cout << "{\"groups\":" << groups << ",\"assertions\":" << checks << "}\n"; return 0;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
