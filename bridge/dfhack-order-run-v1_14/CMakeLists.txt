# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpOrderRunV1_14.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpOrderRunV1_14.proto" COPYONLY)
# Development only: build against one exact matching DFHack source tree.
dfhack_plugin(dfmcp_order_run_v1_14 dfmcp_order_run_v1_14.cpp PROTOBUFS DfmcpOrderRunV1_14
    LINK_LIBRARIES protobuf)
