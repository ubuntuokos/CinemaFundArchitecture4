#!/bin/bash
# usage: conf.sh <stream_dir>  -> prints CSV line
d=$1; n=$(basename $d); V=/home/claude/vvc; T=$(mktemp -d /home/claude/vvc/tmp.XXXX)
bit=$(ls $d/*.bit $d/*.bin 2>/dev/null | head -1)
ref=$(cat $d/*.yuv.md5 2>/dev/null | grep -o '^[0-9a-f]\{32\}' | head -1); nref=$(ls $d/*yuv.md5 2>/dev/null | wc -l)
h(){ [ -s "$1" ] && md5sum "$1" | cut -c1-32 || echo FAIL; }
timeout 1800 $V/src/vtm/bin/DecoderAppStatic -b $bit -o $T/vtm.yuv >$T/vtm.log 2>&1; vtm=$(h $T/vtm.yuv); rm -f $T/vtm.yuv
timeout 900 $V/bin/vvdecapp -b $bit -o $T/vvdec.yuv -t 2 >$T/vvdec.log 2>&1; vvd=$(h $T/vvdec.yuv); rm -f $T/vvdec.yuv
timeout 900 $V/prefix/bin/ffmpeg -nostdin -v error -threads 2 -i $bit -f rawvideo -y $T/ff.yuv >$T/ff.log 2>&1; ff=$(h $T/ff.yuv); rm -f $T/ff.yuv
echo "$n,$nref,$ref,$vtm,$vvd,$ff"
mkdir -p $V/conf/logs/$n && mv $T/*.log $V/conf/logs/$n/ ; rmdir $T
