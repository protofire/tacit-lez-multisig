cmake_minimum_required(VERSION 3.14)
project(MultisigModulePlugin LANGUAGES CXX)

if(DEFINED ENV{LOGOS_MODULE_BUILDER_ROOT})
    include($ENV{LOGOS_MODULE_BUILDER_ROOT}/cmake/LogosModule.cmake)
else()
    message(FATAL_ERROR "LOGOS_MODULE_BUILDER_ROOT is not set.")
endif()

configure_file(${CMAKE_CURRENT_SOURCE_DIR}/metadata.json
               ${CMAKE_CURRENT_BINARY_DIR}/metadata.json COPYONLY)

# pcsc-sys (wallet dependency) leaves SCard* undefined in the Rust staticlib; they are never called
logos_module(NAME multisig_module LINK_LIBRARIES pcsclite)
