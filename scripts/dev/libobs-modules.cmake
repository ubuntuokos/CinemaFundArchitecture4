# SPDX-License-Identifier: GPL-3.0-or-later
# CFA4's allow-list of OBS Studio 33.0.1 core modules (CFA4-LIVE-STUDIO-001-S2, D-S2-1).
#
# scripts/dev/build-libobs-33.0.1.sh copies this file over plugins/CMakeLists.txt of the unpacked
# upstream release before it configures the build. OBS 33 builds all its core modules (and so needs
# the dependencies of every one of them: MbedTLS, SRT, RIST, x264, Qt…) or none; this build wants
# exactly the three capture modules below. It is build glue only: no OBS source file is changed.
# Added modules need an owner-approved plan step (S5 outputs, S7 encoders).

if(NOT TARGET libobs)
  message(FATAL_ERROR "OBS Core modules require 'libobs' target.")
endif()

set_property(GLOBAL APPEND PROPERTY OBS_FEATURES_ENABLED "Core Modules")

add_core_module(linux-pipewire PLATFORMS LINUX)
add_core_module(linux-pulseaudio PLATFORMS LINUX)
add_core_module(linux-v4l2 PLATFORMS LINUX)

set_obs_core_modules()
