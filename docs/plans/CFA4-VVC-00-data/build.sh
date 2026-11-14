#!/bin/bash
set -e
S=/home/claude/vvc/src; P=/home/claude/vvc/prefix; mkdir -p $P
cd $S/vvdec && cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$P >/dev/null && ninja -C build install >/dev/null && echo vvdec ok
cd $S/vvenc && cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON -DCMAKE_INSTALL_PREFIX=$P >/dev/null && ninja -C build install >/dev/null && echo vvenc ok
cd $S/vtm && cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null && ninja -C build DecoderApp EncoderApp >/dev/null && echo vtm ok
cd $S/uvg266 && cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DUVG_BIT_DEPTH=10 -DCMAKE_INSTALL_PREFIX=$P/uvg10 >/dev/null && ninja -C build install >/dev/null && echo uvg266 ok
cd $S/ffmpeg && PKG_CONFIG_PATH=$P/lib/pkgconfig ./configure --prefix=$P --enable-gpl --enable-version3 --enable-libvvenc --disable-doc --disable-debug --extra-ldflags="-Wl,-rpath,$P/lib" >/dev/null && make -j2 >/dev/null 2>&1 && make install >/dev/null && echo ffmpeg ok
