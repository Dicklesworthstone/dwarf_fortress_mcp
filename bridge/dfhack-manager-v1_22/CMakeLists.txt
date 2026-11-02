file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpManagerV1_22.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpManagerV1_22.proto" COPYONLY)
dfhack_plugin(dfmcp_manager_v1_22 dfmcp_manager_v1_22.cpp PROTOBUFS DfmcpManagerV1_22
    LINK_LIBRARIES protobuf)
