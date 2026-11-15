#!/bin/bash
V=/home/claude/vvc; out=$V/enc_results.csv
echo "seq,enc,preset,qp,frames,enc_s,fps,kbps,psnr_y,vvdec_md5,vtm_md5,ffmpeg_md5,uvgrecon_md5" > $out
for s in crowd_run ducks_take_off old_town_cross park_joy; do
 for cfg in "vvenc medium" "vvenc faster" "uvg266 medium" "uvg266 ultrafast"; do
  for qp in 22 27 32 37; do $V/scripts/enc.sh ${s}_1080p_32f_10b 1920x1080 32 $cfg $qp >> $out; done; done; done
for cfg in "vvenc faster" "uvg266 ultrafast"; do $V/scripts/enc.sh crowd_run_2160p_16f_10b 3840x2160 16 $cfg 32 >> $out; done
echo done > $V/ENC_DONE
