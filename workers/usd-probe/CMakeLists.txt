# SPDX-License-Identifier: GPL-3.0-or-later
# cfa4-usd-probe: the G-USD-1 probe worker (docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md, S5, SB-G6).
# Optional: not part of `cargo` builds or CI. It links the minimal OpenUSD core built by scripts/dev/build-usd.sh
# (oneTBB is the only external library) and nothing else. It is NOT the U2 worker: it only answers the questions the
# sandbox and fuzz gate need (inspect, write small test files).
#
#   cmake -S workers/usd-probe -B target/usd-probe -DCFA4_USD_PREFIX=$PWD/target/usd-install
#   cmake --build target/usd-probe
cmake_minimum_required(VERSION 3.22)
project(cfa4_usd_probe CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CFA4_USD_PREFIX "" CACHE PATH "install prefix of scripts/dev/build-usd.sh")
if(NOT CFA4_USD_PREFIX)
  message(FATAL_ERROR "set -DCFA4_USD_PREFIX=<the build-usd.sh install prefix>")
endif()
list(APPEND CMAKE_PREFIX_PATH "${CFA4_USD_PREFIX}")
find_package(pxr REQUIRED CONFIG PATHS "${CFA4_USD_PREFIX}" NO_DEFAULT_PATH)
add_executable(cfa4-usd-probe main.cc)
target_include_directories(cfa4-usd-probe PRIVATE "${CFA4_USD_PREFIX}/include")
target_link_libraries(cfa4-usd-probe PRIVATE usd sdf tf vt gf usdGeom usdUtils)
set_target_properties(cfa4-usd-probe PROPERTIES BUILD_RPATH "${CFA4_USD_PREFIX}/lib" INSTALL_RPATH "$ORIGIN/../lib")
