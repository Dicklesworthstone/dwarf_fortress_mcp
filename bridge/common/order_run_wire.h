#pragma once
#include "order_run.h"
#include "retained_snapshot.h"
#include <string_view>

namespace dfmcp_order_run {
constexpr std::size_t MAX_CAPTURE_BYTES = 573, MAX_PLAN_BYTES = 604, MAX_RECORD_BYTES = 1425;
inline bool utf8(const std::string &s, std::size_t maximum) {
    if (s.empty() || s.size() > maximum) return false;
    for (std::size_t i = 0; i < s.size();) {
        const auto c = static_cast<unsigned char>(s[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || s.size() - i < n) return false;
        for (std::size_t k = 1; k < n; ++k)
            if ((static_cast<unsigned char>(s[i+k]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(s[i+1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void u32(std::string &out, std::uint32_t n) { for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s)); }
inline void u64(std::string &out, std::uint64_t n) { for (int s = 56; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s)); }
inline void field(std::string &out, const std::string &value) {
    require(value.size() <= UINT16_MAX); out.push_back(static_cast<char>(value.size() >> 8));
    out.push_back(static_cast<char>(value.size())); out += value;
}
inline std::string hash(const char *domain, const std::string &value) {
    std::string bytes(domain); bytes.push_back('\0'); bytes += value; return dfmcp_snapshot::sha256(bytes);
}
inline std::string encode_capture(const Capture &v) {
    v.validate(); require(utf8(v.identity.folder, 512), 5);
    std::string out = "DFMOR014";
    for (auto n : {v.clock.generation, v.clock.sequence, v.clock.tick}) u64(out, n);
    u32(out, v.identity.site); field(out, v.identity.folder); out.push_back(v.clock.paused ? 1 : 0);
    u32(out, v.id); u32(out, v.next_order); out.push_back(v.present ? 1 : 0); out.push_back(static_cast<char>(v.recipe));
    u32(out, static_cast<std::uint32_t>(v.total)); u32(out, static_cast<std::uint32_t>(v.left)); u32(out, v.status);
    require(out.size() <= MAX_CAPTURE_BYTES, 5); return out;
}
class Reader {
    std::string_view input;
public:
    explicit Reader(std::string_view value) : input(value) {}
    std::string_view take(std::size_t n) { require(n <= input.size()); auto part = input.substr(0, n); input.remove_prefix(n); return part; }
    std::uint32_t number(std::size_t n) { std::uint32_t v = 0; for (unsigned char c : take(n)) v = (v << 8) | c; return v; }
    std::uint64_t wide() { const auto hi = number(4); return (std::uint64_t{hi} << 32) | number(4); }
    bool boolean() { auto v = number(1); require(v <= 1); return v != 0; }
    std::int32_t signed_number() {
        const auto v = number(4);
        return static_cast<std::int32_t>(v <= INT32_MAX ? static_cast<std::int64_t>(v) : static_cast<std::int64_t>(v) - 4294967296LL);
    }
    std::string text() { auto n = number(2); require(n >= 1 && n <= 512); return std::string(take(n)); }
    void finish() const { require(input.empty()); }
};
inline Capture decode_capture(const std::string &data) {
    require(data.size() <= MAX_CAPTURE_BYTES); Reader r(data); require(r.take(8) == "DFMOR014");
    Capture out; out.clock.generation = r.wide(); out.clock.sequence = r.wide(); out.clock.tick = r.wide();
    out.identity.generation = out.clock.generation; out.identity.site = r.number(4); out.identity.folder = r.text();
    out.clock.loaded = true; out.clock.clock_valid = true; out.clock.paused = r.boolean();
    out.id = r.number(4); out.next_order = r.number(4); out.present = r.boolean(); out.recipe = static_cast<std::uint8_t>(r.number(1));
    out.total = r.signed_number(); out.left = r.signed_number(); out.status = r.number(4); r.finish();
    require(encode_capture(out) == data); return out;
}
inline std::string plan_bytes(clock::Spec limits, Goal goal, const Capture &before) {
    goal.validate(limits, before); require(!goal.matches(before), 4);
    std::string out = "DFMOP014"; u32(out, limits.game_ticks); u32(out, limits.wall_ms);
    out.push_back(static_cast<char>(goal.predicate));
    for (auto n : {goal.threshold, goal.samples, goal.interval}) u32(out, n);
    field(out, encode_capture(before)); require(out.size() <= MAX_PLAN_BYTES, 5); return out;
}
inline std::string plan_digest(clock::Spec limits, Goal goal, const Capture &before) {
    return hash("dfmcp-order-run-plan/1", plan_bytes(limits, goal, before));
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    clock::valid_key(key); require(plan.size() == 32); std::string out; field(out, key); out += plan;
    return hash("dfmcp-order-run-token/1", out).substr(0, 16);
}
inline std::string encode_record(const Record &record) {
    require(record.run != nullptr, 5); const auto &v = *record.run;
    const auto plan = plan_bytes(v.spec, record.goal, record.before);
    require(v.plan == hash("dfmcp-order-run-plan/1", plan), 5);
    std::string out = "DFMOE014"; field(out, v.key); field(out, plan); out += v.plan; out += token_for(v.key, v.plan);
    out.push_back(static_cast<char>(v.phase)); out.push_back(static_cast<char>(v.reason)); out.push_back(static_cast<char>(record.trigger));
    out.push_back(v.unpause_attempted ? 1 : 0); out.push_back(v.pause_verified ? 1 : 0); out.push_back(v.tick_known ? 1 : 0);
    u64(out, v.tick_known ? v.observed_tick : 0); u32(out, record.stable_samples); u64(out, record.counted_tick);
    field(out, record.sample ? encode_capture(*record.sample) : std::string());
    out += hash("dfmcp-order-run-receipt/1", out); require(out.size() <= MAX_RECORD_BYTES, 5); return out;
}
} // namespace dfmcp_order_run
