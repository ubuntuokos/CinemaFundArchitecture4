#!/bin/bash
# DCPOM-1 survey: build DCP-o-matic 2.18.51 command-line tools (no GUI) and their chain into $P.
set -euo pipefail
# Before running (DCPOM-01 audit, docs/audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md):
#   cd $DCPOM_DIR; for r in dcpomatic libdcp libsub libcxml asdcplib leqm-nrt libttf openssl; do
#     git clone https://git.carlh.net/git/$r.git; done       # full clones: the version comes from git tags
#   git clone --depth 1 --branch v8.0.2 https://github.com/fastfloat/fast_float.git
#   mkdir -p prefix/include && cp -r fast_float/include/fast_float prefix/include/   # Ubuntu 24.04's is too old
# After it: cd openssl && git checkout 54298369cacfe0ae01c5aa42ace8a463fd2e7a2e &&
#   ./Configure no-shared no-tests -fPIC && make build_programs && cp apps/openssl ../prefix/bin/dcpomatic2_openssl
D=${DCPOM_DIR:-$HOME/dcpom}
P=$D/prefix
export PKG_CONFIG_PATH=$P/lib64/pkgconfig:$P/lib/pkgconfig:$P/lib/x86_64-linux-gnu/pkgconfig
export LD_LIBRARY_PATH=$P/lib64:$P/lib:$P/lib/x86_64-linux-gnu
export PATH=$P/bin:$PATH
export CXXFLAGS="-O2 -I$P/include"
J=$(nproc)
t() { echo "=== $(date +%T) $*"; }

[ -e $D/.done-apt ] || {
t apt
sudo -n true 2>/dev/null && S=sudo || S=
$S apt-get install -y -qq build-essential pkg-config python3 git cmake \
  libboost-all-dev libxml++2.6-dev libxmlsec1-dev libssl-dev libopenjp2-7-dev \
  libavformat-dev libavcodec-dev libavfilter-dev libswscale-dev libswresample-dev libavutil-dev libavdevice-dev libpostproc-dev \
  libsamplerate0-dev libglib2.0-dev libzip-dev libfontconfig1-dev libpangomm-1.4-dev libcairomm-1.0-dev \
  libcurl4-openssl-dev libssh-dev libicu-dev libsndfile1-dev libmagick++-dev libsigc++-2.0-dev \
  libfmt-dev libnanomsg-dev libsqlite3-dev libpng-dev libjpeg-dev libfreetype-dev libharfbuzz-dev \
  libpolkit-gobject-1-dev libasound2-dev gettext >/dev/null
touch $D/.done-apt; }

waf_build() { # dir, tag, extra configure args
  [ -e $D/.done-$1 ] && return 0
  t "$1 $2"; cd $D/$1; git checkout -q "$2"
  python3 waf configure --prefix=$P $3; python3 waf build -j$J; python3 waf install; touch $D/.done-$1
}
waf_build asdcplib v1.0.10 ""
waf_build libcxml v0.17.17 "--disable-tests"
waf_build libdcp v1.10.64 "--disable-tests --disable-examples --disable-dumpimage"
waf_build libsub v1.6.65 "--disable-tests"
waf_build leqm-nrt 3913040723f746c0b4a59cb55cd8a23030ea49a2 ""
waf_build libttf v0.0.7 ""
waf_build dcpomatic v2.18.51 "--disable-gui --disable-tests"
t done
ls -la $P/bin
du -sh $P
