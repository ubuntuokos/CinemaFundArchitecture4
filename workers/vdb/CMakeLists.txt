# cfa4-vdb-worker: the separate-process OpenVDB reader of CFA4 (plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1).
# Optional: not part of `cargo` builds or CI; needs libopenvdb-dev (Apache-2.0) and libtbb-dev installed by the user.
cmake_minimum_required(VERSION 3.22)
project(cfa4_vdb_worker CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# Ubuntu/Debian ship FindOpenVDB.cmake (no config file) in this directory.
list(APPEND CMAKE_MODULE_PATH /usr/lib/${CMAKE_LIBRARY_ARCHITECTURE}/cmake/OpenVDB /usr/lib/x86_64-linux-gnu/cmake/OpenVDB)
find_package(OpenVDB REQUIRED)
add_executable(cfa4-vdb-worker main.cc)
target_link_libraries(cfa4-vdb-worker PRIVATE OpenVDB::openvdb z)
