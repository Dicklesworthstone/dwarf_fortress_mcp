#pragma once

#include <cstdint>
#include <vector>
#include <string>

namespace dfmcp {

constexpr size_t MAX_FRAME_PAYLOAD_SIZE = 16 * 1024 * 1024; // 16MB
constexpr size_t FRAME_HEADER_SIZE = 10;
constexpr const char* DEFAULT_SOCKET_PATH = "/tmp/dfhack-mcp.sock";

enum class MessageType : uint16_t {
    HandshakeRequest = 1,
    HandshakeResponse = 2,
    HealthRequest = 3,
    HealthResponse = 4,
    ProbeCompatibilityRequest = 5,
    ProbeCompatibilityResponse = 6,
    ReadSnapshotRequest = 7,
    ReadSnapshotResponse = 8,
    ReadDeltaRequest = 9,
    ReadDeltaResponse = 10,
    PrepareMutationRequest = 11,
    PrepareMutationResponse = 12,
    CommitMutationRequest = 13,
    CommitMutationResponse = 14,
    LookupOperationRequest = 15,
    LookupOperationResponse = 16,
    CancelOperationRequest = 17,
    CancelOperationResponse = 18,
    CreateCheckpointRequest = 19,
    CreateCheckpointResponse = 20,
    RestoreCheckpointRequest = 21,
    RestoreCheckpointResponse = 22,
    Heartbeat = 23,
    ErrorResponse = 24
};

// Standard IEEE 802.3 CRC-32 checksum calculation
inline uint32_t compute_crc32(const uint8_t* data, size_t length) {
    uint32_t crc = 0xFFFFFFFFu;
    for (size_t i = 0; i < length; ++i) {
        crc ^= static_cast<uint32_t>(data[i]);
        for (int b = 0; b < 8; ++b) {
            if (crc & 1) {
                crc = (crc >> 1) ^ 0xEDB88320u;
            } else {
                crc >>= 1;
            }
        }
    }
    return ~crc;
}

struct Frame {
    MessageType type;
    std::vector<uint8_t> payload;

    std::vector<uint8_t> encode() const {
        uint32_t len = static_cast<uint32_t>(payload.size());
        uint16_t type_val = static_cast<uint16_t>(type);
        uint32_t crc = compute_crc32(payload.data(), payload.size());

        std::vector<uint8_t> bytes;
        bytes.reserve(FRAME_HEADER_SIZE + payload.size());

        // Big-endian length
        bytes.push_back((len >> 24) & 0xFF);
        bytes.push_back((len >> 16) & 0xFF);
        bytes.push_back((len >> 8) & 0xFF);
        bytes.push_back(len & 0xFF);

        // Big-endian type
        bytes.push_back((type_val >> 8) & 0xFF);
        bytes.push_back(type_val & 0xFF);

        // Big-endian CRC32
        bytes.push_back((crc >> 24) & 0xFF);
        bytes.push_back((crc >> 16) & 0xFF);
        bytes.push_back((crc >> 8) & 0xFF);
        bytes.push_back(crc & 0xFF);

        bytes.insert(bytes.end(), payload.begin(), payload.end());
        return bytes;
    }
};

} // namespace dfmcp
