cmake_minimum_required(VERSION 3.20)
project(dfmcp_bridge CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

message(STATUS "Building compile-only placeholder; no DFHack integration is present")

set(SOURCES
    src/dfmcp_bridge.cpp
)

add_library(dfmcp_bridge SHARED ${SOURCES})

target_include_directories(dfmcp_bridge PRIVATE include)

target_compile_options(dfmcp_bridge PRIVATE
    -Wall
    -Wextra
    -Wno-unused-parameter
    -fPIC
)

set_target_properties(dfmcp_bridge PROPERTIES
    PREFIX ""
    OUTPUT_NAME "dfmcp_bridge"
)
