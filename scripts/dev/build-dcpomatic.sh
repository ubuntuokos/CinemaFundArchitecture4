#!/usr/bin/env bash
# Builds the DCP-o-matic CFA4 pins (plan CFA4-DCPOMATIC-001 D8/D10): the command-line tools of
# DCP-o-matic 2.18.51, no GUI, their libraries, and the patched OpenSSL `dcpomatic2_openssl` next
# to the tools. Same chain as the DCPOM-01 audit (docs/audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md).
#
#   DCPOM_DIR=~/dcpom scripts/dev/build-dcpomatic.sh
#   export CFA4_DCPOMATIC_BIN_DIR=$DCPOM_DIR/prefix/bin
#   export LD_LIBRARY_PATH=$DCPOM_DIR/prefix/lib64:$DCPOM_DIR/prefix/lib
#
# Every source is checked out at a fixed commit and the commit is verified. Steps already done are
# skipped (marker files), so a cached $DCPOM_DIR rebuilds nothing. Installs build packages with
# apt unless DCPOM_SKIP_APT=1.
set -euo pipefail
D=${DCPOM_DIR:-$HOME/dcpom}
P=$D/prefix
mkdir -p "$D" "$P/include" "$P/bin"
export PKG_CONFIG_PATH=$P/lib64/pkgconfig:$P/lib/pkgconfig:$P/lib/x86_64-linux-gnu/pkgconfig
export LD_LIBRARY_PATH=$P/lib64:$P/lib:$P/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export PATH=$P/bin:$PATH
# fast_float first: Ubuntu 24.04's is too old for libdcp
export CXXFLAGS="-O2 -I$P/include"
J=$(nproc)
t() { echo "=== $(date +%T) $*"; }

if [[ "${DCPOM_SKIP_APT:-0}" != 1 && ! -e $D/.done-apt ]]; then
  t apt
  S=; [[ $(id -u) == 0 ]] || S=sudo
  $S apt-get update -qq
  $S apt-get install -y -qq --no-install-recommends build-essential pkg-config python3 git cmake perl \
    libboost-all-dev libxml++2.6-dev libxmlsec1-dev libssl-dev libopenjp2-7-dev \
    libavformat-dev libavcodec-dev libavfilter-dev libswscale-dev libswresample-dev libavutil-dev libavdevice-dev libpostproc-dev \
    libsamplerate0-dev libglib2.0-dev libzip-dev libfontconfig1-dev libpangomm-1.4-dev libcairomm-1.0-dev \
    libcurl4-openssl-dev libssh-dev libicu-dev libsndfile1-dev libsigc++-2.0-dev libxerces-c-dev \
    libfmt-dev libnanomsg-dev libsqlite3-dev libpng-dev libjpeg-dev libfreetype-dev libharfbuzz-dev \
    libpolkit-gobject-1-dev libasound2-dev gettext >/dev/null
  touch "$D/.done-apt"
fi

# name, url, commit. Full clones: the version strings come from `git describe` (a shallow
# clone printed "2.18.51 fatal: amb" in the audit).
fetch() {
  local name=$1 url=$2 commit=$3
  if [[ ! -d $D/$name/.git ]]; then
    t "fetch $name"
    git clone -q "$url" "$D/$name"
  fi
  git -C "$D/$name" -c advice.detachedHead=false checkout -q "$commit"
  [[ $(git -C "$D/$name" rev-parse HEAD) == "$commit" ]] || { echo "$name is not at $commit" >&2; exit 1; }
}
fetch asdcplib https://git.carlh.net/git/asdcplib.git 98aa300e42aff7078cc767af14712200362a1843  # v1.0.10
fetch libcxml https://git.carlh.net/git/libcxml.git efde22cde088969f7f7edb23593bbb22a65f5eea    # v0.17.17
fetch libdcp https://git.carlh.net/git/libdcp.git 891facc8ce17cd767948ac5c2eb629bca5288f64      # v1.10.64
fetch libsub https://git.carlh.net/git/libsub.git 60c9a08ff01c354468e575c7229638368f286726      # v1.6.65
fetch leqm-nrt https://git.carlh.net/git/leqm-nrt.git 3913040723f746c0b4a59cb55cd8a23030ea49a2
fetch libttf https://git.carlh.net/git/libttf.git 6d830c63ba53993f2818b4fb61dbc17284f8f557      # v0.0.7
fetch openssl https://git.carlh.net/git/openssl.git 54298369cacfe0ae01c5aa42ace8a463fd2e7a2e    # 3.2.0 + notBefore patch
fetch dcpomatic https://github.com/cth103/dcpomatic.git 920fb9b04461eb28558407dd76611a751942e4ba  # v2.18.51
[[ $(git -C "$D/dcpomatic" describe --tags) == v2.18.51 ]] || { echo "dcpomatic: tag v2.18.51 missing" >&2; exit 1; }
if [[ ! -d $P/include/fast_float ]]; then
  t fast_float
  fetch fast_float https://github.com/fastfloat/fast_float.git 50a80a73ab2ab256ba1c3bf86923ddd8b4202bc7  # v8.0.2
  cp -r "$D/fast_float/include/fast_float" "$P/include/"
fi

waf_build() { # dir, extra configure args
  [[ -e $D/.done-$1 ]] && return 0
  t "build $1"
  (cd "$D/$1" && python3 waf configure --prefix="$P" $2 && python3 waf build -j"$J" && python3 waf install)
  touch "$D/.done-$1"
}
waf_build asdcplib ""
waf_build libcxml "--disable-tests"
waf_build libdcp "--disable-tests --disable-examples --disable-dumpimage"
waf_build libsub "--disable-tests"
waf_build leqm-nrt ""
waf_build libttf ""
waf_build dcpomatic "--disable-gui --disable-tests"

if [[ ! -x $P/bin/dcpomatic2_openssl ]]; then
  t "build dcpomatic2_openssl"
  (cd "$D/openssl" && ./Configure no-shared no-tests -fPIC >/dev/null && make -s -j"$J" build_programs >/dev/null && cp apps/openssl "$P/bin/dcpomatic2_openssl")
fi

t "check"
v=$("$P/bin/dcpomatic2_cli" --version | head -n1)
echo "$v"
[[ $v == "dcpomatic version 2.18.51"* && $v != *fatal* ]] || { echo "unexpected version: $v" >&2; exit 1; }
du -sh "$P"
