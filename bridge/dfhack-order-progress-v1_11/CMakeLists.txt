# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpOrderProgressV1_11.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpOrderProgressV1_11.proto" COPYONLY)
# Add this directory to an exact, matching DFHack source build for development.
dfhack_plugin(dfmcp_order_progress_v1_11 dfmcp_order_progress_v1_11.cpp PROTOBUFS DfmcpOrderProgressV1_11
    LINK_LIBRARIES protobuf)
