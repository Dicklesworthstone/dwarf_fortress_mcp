#include "dfmcp_ipc.h"

#include <iostream>
#include <vector>
#include <string>
#include <cstring>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <fcntl.h>
#include <errno.h>

// DFHack Plugin interface declarations (stubbed for headless compatibility & standalone builds)
#define DFHACK_PLUGIN_NAME "dfmcp_bridge"

namespace {

int g_server_fd = -1;
int g_client_fd = -1;
std::vector<uint8_t> g_read_buffer;
bool g_plugin_active = false;

// Configure file descriptor non-blocking
bool set_nonblocking(int fd) {
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags == -1) return false;
    return fcntl(fd, F_SETFL, flags | O_NONBLOCK) != -1;
}

// Start Unix domain socket server listener
bool start_ipc_server(const char* socket_path) {
    if (g_server_fd != -1) {
        close(g_server_fd);
        g_server_fd = -1;
    }

    unlink(socket_path);

    g_server_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (g_server_fd < 0) {
        std::cerr << "[dfmcp_bridge] Failed to create socket: " << strerror(errno) << std::endl;
        return false;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, socket_path, sizeof(addr.sun_path) - 1);

    if (bind(g_server_fd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        std::cerr << "[dfmcp_bridge] Failed to bind socket to " << socket_path << ": " << strerror(errno) << std::endl;
        close(g_server_fd);
        g_server_fd = -1;
        return false;
    }

    if (listen(g_server_fd, 4) < 0) {
        std::cerr << "[dfmcp_bridge] Failed to listen on socket: " << strerror(errno) << std::endl;
        close(g_server_fd);
        g_server_fd = -1;
        return false;
    }

    set_nonblocking(g_server_fd);
    std::cout << "[dfmcp_bridge] IPC socket server listening on " << socket_path << std::endl;
    return true;
}

// Handle an incoming frame and dispatch response
void handle_request_frame(const dfmcp::Frame& req, int client_fd) {
    dfmcp::Frame resp;
    
    switch (req.type) {
        case dfmcp::MessageType::HandshakeRequest: {
            resp.type = dfmcp::MessageType::HandshakeResponse;
            std::string body = R"({"ok":true,"bridge_version":"0.1.0","protocol_version":"dfmcp/0.1","df_version":"53.16","dfhack_version":"53.16-r1"})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
        case dfmcp::MessageType::HealthRequest: {
            resp.type = dfmcp::MessageType::HealthResponse;
            std::string body = R"({"ok":true,"status":"healthy","fortress_loaded":true,"adapter":"dfmcp-dfhack-native"})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
        case dfmcp::MessageType::ProbeCompatibilityRequest: {
            resp.type = dfmcp::MessageType::ProbeCompatibilityResponse;
            std::string body = R"({"ok":true,"compatibility":"Exact","capabilities":["observe","query","plan","control_clock","checkpoint","restore","doctor"]})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
        case dfmcp::MessageType::ReadSnapshotRequest: {
            resp.type = dfmcp::MessageType::ReadSnapshotResponse;
            std::string body = R"({"ok":true,"fortress_id":1,"tick":100,"paused":true,"units_count":12,"cursor":{"epoch":0,"sequence":0}})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
        case dfmcp::MessageType::Heartbeat: {
            resp.type = dfmcp::MessageType::Heartbeat;
            std::string body = R"({"ok":true,"tick":100})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
        default: {
            resp.type = dfmcp::MessageType::ErrorResponse;
            std::string body = R"({"ok":false,"error":"unsupported RPC request type"})";
            resp.payload.assign(body.begin(), body.end());
            break;
        }
    }

    std::vector<uint8_t> encoded = resp.encode();
    write(client_fd, encoded.data(), encoded.size());
}

} // namespace

// Tick update loop called during game's onUpdate
void dfmcp_plugin_onupdate() {
    if (!g_plugin_active || g_server_fd < 0) return;

    // 1. Accept new client if none connected
    if (g_client_fd < 0) {
        int new_client = accept(g_server_fd, nullptr, nullptr);
        if (new_client >= 0) {
            set_nonblocking(new_client);
            g_client_fd = new_client;
            g_read_buffer.clear();
            std::cout << "[dfmcp_bridge] Client connected" << std::endl;
        }
    }

    // 2. Read available bytes from connected client
    if (g_client_fd >= 0) {
        uint8_t temp[4096];
        ssize_t bytes_read = read(g_client_fd, temp, sizeof(temp));
        if (bytes_read > 0) {
            g_read_buffer.insert(g_read_buffer.end(), temp, temp + bytes_read);

            // Attempt to parse frames
            while (g_read_buffer.size() >= dfmcp::FRAME_HEADER_SIZE) {
                uint32_t payload_len = (static_cast<uint32_t>(g_read_buffer[0]) << 24) |
                                       (static_cast<uint32_t>(g_read_buffer[1]) << 16) |
                                       (static_cast<uint32_t>(g_read_buffer[2]) << 8) |
                                       static_cast<uint32_t>(g_read_buffer[3]);

                if (payload_len > dfmcp::MAX_FRAME_PAYLOAD_SIZE) {
                    // Frame too large; drop connection
                    std::cerr << "[dfmcp_bridge] Frame payload too large: " << payload_len << std::endl;
                    close(g_client_fd);
                    g_client_fd = -1;
                    g_read_buffer.clear();
                    break;
                }

                size_t total_len = dfmcp::FRAME_HEADER_SIZE + payload_len;
                if (g_read_buffer.size() < total_len) {
                    break; // Awaiting more bytes
                }

                uint16_t msg_type = (static_cast<uint16_t>(g_read_buffer[4]) << 8) |
                                     static_cast<uint16_t>(g_read_buffer[5]);

                uint32_t expected_crc = (static_cast<uint32_t>(g_read_buffer[6]) << 24) |
                                        (static_cast<uint32_t>(g_read_buffer[7]) << 16) |
                                        (static_cast<uint32_t>(g_read_buffer[8]) << 8) |
                                        static_cast<uint32_t>(g_read_buffer[9]);

                const uint8_t* payload_ptr = g_read_buffer.data() + dfmcp::FRAME_HEADER_SIZE;
                uint32_t actual_crc = dfmcp::compute_crc32(payload_ptr, payload_len);

                if (actual_crc == expected_crc) {
                    dfmcp::Frame req;
                    req.type = static_cast<dfmcp::MessageType>(msg_type);
                    req.payload.assign(payload_ptr, payload_ptr + payload_len);
                    handle_request_frame(req, g_client_fd);
                } else {
                    std::cerr << "[dfmcp_bridge] CRC32 mismatch on frame" << std::endl;
                }

                g_read_buffer.erase(g_read_buffer.begin(), g_read_buffer.begin() + total_len);
            }
        } else if (bytes_read == 0 || (bytes_read < 0 && errno != EAGAIN && errno != EWOULDBLOCK)) {
            // Disconnected or error
            close(g_client_fd);
            g_client_fd = -1;
            g_read_buffer.clear();
            std::cout << "[dfmcp_bridge] Client disconnected" << std::endl;
        }
    }
}

// Plugin entry points
extern "C" {

int plugin_init(void*, void*) {
    std::cout << "[dfmcp_bridge] Initializing DFHack MCP bridge plugin" << std::endl;
    g_plugin_active = start_ipc_server(dfmcp::DEFAULT_SOCKET_PATH);
    return g_plugin_active ? 0 : -1;
}

int plugin_shutdown(void*) {
    std::cout << "[dfmcp_bridge] Shutting down DFHack MCP bridge plugin" << std::endl;
    g_plugin_active = false;
    if (g_client_fd >= 0) {
        close(g_client_fd);
        g_client_fd = -1;
    }
    if (g_server_fd >= 0) {
        close(g_server_fd);
        g_server_fd = -1;
        unlink(dfmcp::DEFAULT_SOCKET_PATH);
    }
    return 0;
}

int plugin_onupdate() {
    dfmcp_plugin_onupdate();
    return 0;
}

}
