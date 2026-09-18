#include <iostream>
#include <stdexcept>
#include "dfhack_stubs.h"
#ifndef DFMCP_CONTROL_SOURCE
#define DFMCP_CONTROL_SOURCE "../../../bridge/dfhack-control-v1_7/dfmcp_control_v1_7.cpp"
#endif
#include DFMCP_CONTROL_SOURCE

#define CHECK(expression) do { if (!(expression)) throw std::runtime_error(#expression); } while (false)
namespace {
DFHack::color_ostream output;
using Fence = dfmcp_pause::DispatchFence;
void reset(bool paused = true) {
    records.clear(); dispatch_fence = Fence{}; generation = 7;
    fake::paused = paused; fake::loaded = fake::fortress = true;
    fake::year = 105; fake::tick = 3; fake::setters = 0; fake::fault = fake::Fault::None;
}
wire::Request request(const std::string &key, bool paused = false) {
    wire::Request in;
    in.set_bearer_token(std::string(32, 't')); in.set_client_nonce(std::string(16, 'n'));
    in.set_protocol_major(1); in.set_protocol_minor(7);
    in.set_idempotency_key(key); in.set_plan_digest(std::string(32, 'd'));
    in.set_paused(paused); in.set_expected_game_tick(std::uint64_t(fake::year) * 403200 + fake::tick);
    return in;
}
using Handler = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
wire::Reply invoke(Handler fn, const wire::Request &in) {
    wire::Reply out; CHECK(fn(output, &in, &out) == CR_OK); return out;
}
wire::Request prepared(const std::string &key, bool paused = false) {
    auto in = request(key, paused); const auto out = invoke(PreparePause, in);
    CHECK(out.accepted()); CHECK(out.prepare_token().size() == 16);
    in.set_prepare_token(out.prepare_token()); return in;
}

void stale_preparations_cannot_override_newer_effects() {
    for (bool initial : {false, true}) for (bool first_target : {false, true})
        for (bool second_target : {false, true}) for (bool reverse : {false, true}) {
        reset(initial); auto first = prepared("first", first_target), second = prepared("second", second_target);
        const auto &winner = reverse ? second : first;
        const auto &loser = reverse ? first : second;
        CHECK(invoke(CommitPause, winner).accepted()); CHECK(fake::setters == 1);
        CHECK(!invoke(CommitPause, loser).accepted()); CHECK(fake::setters == 1);
        CHECK(fake::paused == winner.paused());
        // Retained query evidence is not a fabricated native not-applied receipt.
        const auto unknown = invoke(QueryPause, loser);
        CHECK(unknown.accepted() && unknown.effect_known() && !unknown.has_receipt_digest());
    }
}
void same_tick_aba_and_noop_effects_advance_the_fence() {
    reset(); auto old = prepared("old"), down = prepared("down");
    CHECK(invoke(CommitPause, down).accepted()); auto up = prepared("up", true);
    CHECK(invoke(CommitPause, up).accepted()); CHECK(fake::paused);
    CHECK(!invoke(CommitPause, old).accepted()); CHECK(fake::setters == 2);
    reset(); old = prepared("old"); const auto noop = prepared("noop", true);
    CHECK(invoke(CommitPause, noop).accepted()); CHECK(!invoke(CommitPause, old).accepted());
    CHECK(fake::setters == 1 && fake::paused);
}
void expiry_is_monotonic_and_prepare_replay_does_not_renew_it() {
    Fence fence; const auto at = Fence::Clock::time_point{} + std::chrono::seconds(100);
    auto before = fence.prepare(10, true, at);
    CHECK(fence.claim(before, 11, true, at + std::chrono::seconds(59)));
    CHECK(!fence.claim(before, 11, true, at + std::chrono::seconds(59)));
    auto expired = fence.prepare(10, true, at);
    CHECK(!fence.claim(expired, 10, true, at + std::chrono::seconds(60)));
    CHECK(!fence.claim(expired, 10, true, at));
    auto reversed = fence.prepare(10, true, at);
    CHECK(!fence.claim(reversed, 10, true, at - std::chrono::seconds(1)));
    CHECK(!fence.claim(reversed, 10, true, at));
    auto ancient = fence.prepare(10, true, Fence::Clock::time_point::min());
    CHECK(!fence.claim(ancient, 10, true, Fence::Clock::time_point::max()));
    auto last = fence.prepare(10, true, Fence::Clock::time_point::max());
    CHECK(fence.claim(last, 10, true, Fence::Clock::time_point::max()));
    reset(); auto in = prepared("expired");
    records.at("expired").guard.created -= std::chrono::seconds(60);
    const auto created = records.at("expired").guard.created;
    CHECK(invoke(PreparePause, in).prepare_token() == in.prepare_token());
    CHECK(records.at("expired").guard.created == created);
    CHECK(!invoke(CommitPause, in).accepted()); CHECK(fake::setters == 0);
}
void external_pause_change_and_bad_clocks_retire_preparations() {
    reset(); const auto in = prepared("external"); fake::paused = false;
    CHECK(!invoke(CommitPause, in).accepted()); fake::paused = true;
    CHECK(!invoke(CommitPause, in).accepted()); CHECK(fake::setters == 0);
    for (bool malformed : {false, true}) {
        reset(); const auto clock = prepared("clock"); fake::tick = malformed ? 403200 : 2;
        CHECK(!invoke(CommitPause, clock).accepted()); fake::tick = 3;
        CHECK(!invoke(CommitPause, clock).accepted()); CHECK(fake::setters == 0);
    }
    reset(); const auto progress = prepared("normal-progress"); ++fake::tick;
    CHECK(invoke(CommitPause, progress).accepted()); CHECK(fake::setters == 1);
}
void setter_and_readback_failures_remain_unknown_without_redispatch() {
    for (auto fault : {fake::Fault::SetterBefore, fake::Fault::SetterAfter, fake::Fault::Readback, fake::Fault::ClockAfter}) {
        reset(); auto old = prepared("old"), failing = prepared("failing"); fake::fault = fault;
        const auto reply = invoke(CommitPause, failing);
        CHECK(reply.accepted() && reply.effect_known() && !reply.effect_applied());
        CHECK(!reply.has_receipt_digest()); CHECK(fake::setters == 1);
        fake::fault = fake::Fault::None;
        CHECK(!invoke(CommitPause, failing).has_receipt_digest()); CHECK(fake::setters == 1);
        CHECK(!invoke(QueryPause, failing).has_receipt_digest());
        CHECK(!invoke(CommitPause, old).accepted()); CHECK(fake::setters == 1);
    }
}
void observed_setter_failure_keeps_a_matching_not_applied_receipt() {
    reset(); auto old = prepared("old"), failing = prepared("failing"); fake::fault = fake::Fault::NoEffect;
    const auto reply = invoke(CommitPause, failing);
    CHECK(reply.accepted() && !reply.effect_applied() && reply.paused());
    CHECK(reply.receipt_digest().size() == 32);
    CHECK(reply.receipt_digest() == receipt("failing", failing.plan_digest(), false, reply.observed_game_tick()));
    fake::fault = fake::Fault::None;
    CHECK(invoke(CommitPause, failing).receipt_digest() == reply.receipt_digest());
    CHECK(!invoke(CommitPause, old).accepted()); CHECK(fake::setters == 1);
}
void duplicate_receipts_remain_historical_after_new_effects_and_expiry() {
    reset(); auto a = prepared("a"); const auto first = invoke(CommitPause, a);
    CHECK(first.effect_applied()); auto b = prepared("b", true); CHECK(invoke(CommitPause, b).effect_applied());
    records.at("a").guard.created -= std::chrono::hours(1);
    const auto repeated = invoke(CommitPause, a);
    CHECK(repeated.receipt_digest() == first.receipt_digest()); CHECK(repeated.paused() == first.paused());
    CHECK(fake::paused && fake::setters == 2);
    CHECK(invoke(PreparePause, a).prepare_token() == a.prepare_token());
}
void every_world_or_map_transition_invalidates_prior_tokens() {
    for (auto event : {SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_MAP_LOADED, SC_MAP_UNLOADED}) {
        reset(); auto in = prepared("old"); CHECK(plugin_onstatechange(output, event) == CR_OK);
        CHECK(generation == 8 && records.empty()); CHECK(!invoke(CommitPause, in).accepted());
        CHECK(!invoke(QueryPause, in).effect_known()); CHECK(fake::setters == 0);
        const auto fresh = prepared("old"); CHECK(fresh.prepare_token() != in.prepare_token());
    }
    reset(); const auto unchanged = prepared("same"); plugin_onstatechange(output, SC_PAUSED);
    CHECK(generation == 7); CHECK(invoke(CommitPause, unchanged).accepted());
    reset(); auto in = prepared("overflow"); generation = std::numeric_limits<std::uint64_t>::max();
    plugin_onstatechange(output, SC_MAP_UNLOADED); CHECK(records.empty());
    CHECK(!invoke(PreparePause, in).accepted()); CHECK(!invoke(CommitPause, in).accepted());
}
void unavailable_world_does_not_reactivate_the_same_preparation() {
    reset(); const auto in = prepared("unloaded"); fake::loaded = false;
    CHECK(!invoke(CommitPause, in).accepted()); fake::loaded = true;
    CHECK(!invoke(CommitPause, in).accepted()); CHECK(fake::setters == 0);
}
void query_only_authority_and_identity_checks_precede_effects() {
    reset(); auto in = request("read-only"); in.set_query_only(true);
    CHECK(!invoke(PreparePause, in).accepted()); CHECK(records.empty());
    in = prepared("valid"); const auto original = in; in.set_query_only(true);
    CHECK(!invoke(CommitPause, in).accepted()); CHECK(fake::setters == 0);
    CHECK(invoke(QueryPause, in).accepted());
    in = original; in.set_prepare_token(std::string(16, 'x'));
    CHECK(!invoke(CommitPause, in).accepted());
    in = original; in.set_bearer_token(std::string(32, 'x'));
    CHECK(!invoke(CommitPause, in).accepted());
    CHECK(fake::setters == 0); CHECK(invoke(CommitPause, original).effect_applied());
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));
    CHECK(service->methods == std::vector<std::string>({"Handshake", "PreparePause", "CommitPause", "QueryPause"}));
}
}
int main() {
    using Test = void (*)();
    const std::vector<std::pair<const char *, Test>> tests = {
        {"stale_preparations", stale_preparations_cannot_override_newer_effects},
        {"same_tick_aba_noop", same_tick_aba_and_noop_effects_advance_the_fence},
        {"monotonic_expiry", expiry_is_monotonic_and_prepare_replay_does_not_renew_it},
        {"changed_preconditions", external_pause_change_and_bad_clocks_retire_preparations},
        {"ambiguous_failures", setter_and_readback_failures_remain_unknown_without_redispatch},
        {"known_not_applied", observed_setter_failure_keeps_a_matching_not_applied_receipt},
        {"terminal_replay", duplicate_receipts_remain_historical_after_new_effects_and_expiry},
        {"map_incarnations", every_world_or_map_transition_invalidates_prior_tokens},
        {"world_unavailable", unavailable_world_does_not_reactivate_the_same_preparation},
        {"read_only_and_identity", query_only_authority_and_identity_checks_precede_effects},
    };
    for (const auto &test : tests) {
        try { test.second(); }
        catch (const std::exception &error) {
            std::cerr << test.first << ": " << error.what() << '\n'; return 1;
        }
    }
    std::cout << "{\"native_handler_scenarios\":" << tests.size()
              << ",\"dfhack_runtime\":false,\"protobuf_runtime\":false,\"rust_runtime\":false}\n";
}
