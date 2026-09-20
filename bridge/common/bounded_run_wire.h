#pragma once

#include "bounded_run.h"
#include "retained_snapshot.h"

// Canonical application bytes for the isolated, unadmitted run/1.13 profile.
// Protobuf is only the envelope. Digests never depend on protobuf serialization.
namespace dfmcp_bounded_run {
inline void append_u32(std::string &out, std::uint32_t value) {
    for (int shift = 24; shift >= 0; shift -= 8) out.push_back(static_cast<char>(value >> shift));
}
inline void append_u64(std::string &out, std::uint64_t value) {
    for (int shift = 56; shift >= 0; shift -= 8) out.push_back(static_cast<char>(value >> shift));
}
inline void append_key(std::string &out, const std::string &key) {
    valid_key(key);
    out.push_back(static_cast<char>(key.size() >> 8));
    out.push_back(static_cast<char>(key.size())); out += key;
}
inline std::string hash_domain(const char *domain, const std::string &bytes) {
    std::string input(domain); input.push_back('\0'); input += bytes;
    return dfmcp_snapshot::sha256(input);
}
inline std::string encode_snapshot(const Snapshot &value) {
    require(value.generation && value.generation != UINT64_MAX, 5);
    require(value.loaded || (!value.clock_valid && !value.paused), 5);
    require(value.clock_valid ? value.tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199
        : value.tick == 0, 5);
    std::string out = "DFMRO013";
    append_u64(out, value.generation); append_u64(out, value.sequence); append_u64(out, value.tick);
    out.push_back(value.loaded ? 1 : 0); out.push_back(value.clock_valid ? 1 : 0);
    out.push_back(value.paused ? 1 : 0); return out;
}
inline Snapshot decode_snapshot(const std::string &bytes) {
    require(bytes.size() == 35 && bytes.compare(0, 8, "DFMRO013") == 0);
    const auto word = [&bytes](std::size_t offset) {
        std::uint64_t value = 0;
        for (std::size_t i = 0; i < 8; ++i)
            value = (value << 8) | static_cast<unsigned char>(bytes[offset + i]);
        return value;
    };
    for (std::size_t i = 32; i < 35; ++i) require(static_cast<unsigned char>(bytes[i]) <= 1);
    Snapshot out{word(8), word(16), word(24), bytes[32] != 0, bytes[33] != 0, bytes[34] != 0};
    require(encode_snapshot(out) == bytes); return out;
}
inline std::string digest_plan(Spec spec, const Snapshot &before) {
    spec.validate(); std::string bytes;
    append_u32(bytes, spec.game_ticks); append_u32(bytes, spec.wall_ms); bytes += encode_snapshot(before);
    return hash_domain("dfmcp-bounded-run-plan/1", bytes);
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    require(plan.size() == 32); std::string bytes; append_key(bytes, key); bytes += plan;
    // This is a commitment, not a second authentication credential.
    return hash_domain("dfmcp-bounded-run-token/1", bytes).substr(0, 16);
}
inline std::string encode_record(const Record &record) {
    require(record.plan == digest_plan(record.spec, record.before), 7);
    std::string out = "DFMRE013"; append_key(out, record.key);
    append_u32(out, record.spec.game_ticks); append_u32(out, record.spec.wall_ms);
    out += encode_snapshot(record.before); out += record.plan; out += token_for(record.key, record.plan);
    out.push_back(static_cast<char>(record.phase)); out.push_back(static_cast<char>(record.reason));
    out.push_back(record.unpause_attempted ? 1 : 0); out.push_back(record.pause_verified ? 1 : 0);
    out.push_back(record.tick_known ? 1 : 0); append_u64(out, record.tick_known ? record.observed_tick : 0);
    // Also present for pending records: an integrity digest is NOT a completion proof.
    out += hash_domain("dfmcp-bounded-run-receipt/1", out);
    require(out.size() <= 374, 5); return out;
}
} // namespace dfmcp_bounded_run
