cmake_minimum_required(VERSION 3.20)
project(dfmcp_bridge CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT DEFINED DFHACK_DIR)
    message(STATUS "DFHACK_DIR not set; using local development headers")
endif()

include_directories(
    include
    ${DFHACK_DIR}/include
)

set(SOURCES
    src/dfmcp_bridge.cpp
)

add_library(dfmcp_bridge SHARED ${SOURCES})

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
