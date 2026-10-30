# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpWorkOrdersV1_10.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpWorkOrdersV1_10.proto" COPYONLY)
# Add this directory to an exact, matching DFHack source build for development.
dfhack_plugin(dfmcp_work_orders_v1_10 dfmcp_work_orders_v1_10.cpp PROTOBUFS DfmcpWorkOrdersV1_10
    LINK_LIBRARIES protobuf)
