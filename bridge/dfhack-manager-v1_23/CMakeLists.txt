file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpManagerV1_23.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpManagerV1_23.proto" COPYONLY)
dfhack_plugin(dfmcp_manager_v1_23 dfmcp_manager_v1_23.cpp PROTOBUFS DfmcpManagerV1_23
    LINK_LIBRARIES protobuf)
