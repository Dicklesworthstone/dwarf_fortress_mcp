# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpWorkOrderProgressV1_12.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpWorkOrderProgressV1_12.proto" COPYONLY)
# Standalone unadmitted read-only profile; no existing plugin generation is changed.
dfhack_plugin(dfmcp_work_order_progress_v1_12 dfmcp_work_order_progress_v1_12.cpp PROTOBUFS DfmcpWorkOrderProgressV1_12
    LINK_LIBRARIES protobuf)
