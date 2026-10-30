# Stage the canonical schema where upstream DFHACK_PLUGIN generates it.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpJobControlV1_9.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpJobControlV1_9.proto" COPYONLY)
include_directories(${CMAKE_CURRENT_BINARY_DIR})
dfhack_plugin(dfmcp_job_control_v1_9 dfmcp_job_control_v1_9.cpp PROTOBUFS DfmcpJobControlV1_9
    LINK_LIBRARIES protobuf)
