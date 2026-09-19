#pragma once
// Reuse the checked-in explicit game boundary; never claim this is the SDK.
#include "../work_orders/stubs.h"
namespace dfmcp::order_progress::v1_11 {
#define FIELD(type,name) \
private: type name##_{}; bool has_##name##_ = false; \
public: const type &name() const { return name##_; } bool has_##name() const { return has_##name##_; } \
void set_##name(type value) { name##_ = std::move(value); has_##name##_ = true; } \
void clear_##name() { name##_ = {}; has_##name##_ = false; }
struct UnknownFields { bool present = false; bool empty() const { return !present; } };
struct Request {
    FIELD(std::string,bearer_token)
    FIELD(std::string,client_nonce)
    FIELD(std::uint32_t,protocol_major)
    FIELD(std::uint32_t,protocol_minor)
    FIELD(std::uint32_t,native_order_id)
    UnknownFields unknown;
    struct Reflection { const UnknownFields &GetUnknownFields(const Request &in) const { return in.unknown; } } reflection;
    const Reflection *GetReflection() const { return &reflection; }
    bool IsInitialized() const { return has_bearer_token() && has_client_nonce() && has_protocol_major() && has_protocol_minor(); }
};
inline bool fail_observation_reply = false;
struct Reply {
    FIELD(bool,accepted)
    FIELD(std::uint32_t,failure_code)
    FIELD(std::string,client_nonce)
    FIELD(std::uint32_t,protocol_major)
    FIELD(std::uint32_t,protocol_minor)
    FIELD(std::uint64_t,bridge_generation)
    FIELD(std::string,df_version)
    FIELD(std::string,dfhack_version)
private: std::string observation_; bool has_observation_ = false;
public:
    const std::string &observation() const { return observation_; }
    bool has_observation() const { return has_observation_; }
    void set_observation(const std::string &value) {
        if (fail_observation_reply) { fail_observation_reply = false; throw std::bad_alloc(); }
        observation_ = value; has_observation_ = true;
    }
    void Clear() { *this = Reply(); }
};
#undef FIELD
}
