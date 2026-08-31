project(dfmcp_bridge)

set(PROJECT_SRCS
    src/dfmcp_bridge.cpp
)

set(PROJECT_HDRS
    include/dfmcp_ipc.h
)

set(PROJECT_PROTO
    DfmcpBridge
)

# Build as a genuine DFHack plugin and let DFHack generate/link protobuf Lite
# messages from proto/DfmcpBridge.proto. Protocol v1 deliberately exposes only
# Handshake and ReadObservation.
dfhack_plugin(
    dfmcp_bridge
    ${PROJECT_SRCS}
    LINK_LIBRARIES ${PROJECT_LIBS}
    PROTOBUFS ${PROJECT_PROTO}
)

if(BUILD_PLUGINS)
    target_include_directories(dfmcp_bridge PRIVATE include)
endif()
