#!/bin/bash
# usage: enc.sh <seq> <WxH> <frames> <encoder> <preset> <qp>  -> CSV: seq,enc,preset,qp,frames,enc_s,fps,kbps,psnr_y,vvdec_md5,vtm_md5,ffmpeg_md5,uvgrecon_md5
seq=$1; res=$2; nf=$3; enc=$4; pre=$5; qp=$6; V=/home/claude/vvc; N=$V/nat; O=$V/out; mkdir -p $O
src=$N/$seq.yuv; tag=${seq}_${enc}_${pre}_q$qp; bs=$O/$tag.266; rec=""
t0=$(date +%s.%N)
if [ $enc = vvenc ]; then
  LD_LIBRARY_PATH=$V/prefix/lib $V/prefix/bin/vvencapp -i $src -s $res -r 50 -c yuv420_10 --preset $pre -q $qp -t 2 -f $nf -o $bs > $O/$tag.log 2>&1
else
  $V/prefix/uvg10/bin/uvg266 -i $src --input-res $res --input-fps 50 --input-bitdepth 10 --preset $pre --qp $qp --threads 2 -n $nf -o $bs --debug $O/$tag.rec.yuv > $O/$tag.log 2>&1
fi
t1=$(date +%s.%N)
es=$(echo "$t1 - $t0" | bc); fps=$(echo "scale=3; $nf / $es" | bc)
kbps=$(echo "scale=1; $(stat -c%s $bs) * 8 * 50 / $nf / 1000" | bc)
$V/bin/vvdecapp -b $bs -o $O/$tag.dec.yuv -t 2 >/dev/null 2>&1
vd=$(md5sum < $O/$tag.dec.yuv | cut -c1-32)
$V/src/vtm/bin/DecoderAppStatic -b $bs -o $O/$tag.vtm.yuv -d 10 >/dev/null 2>&1; vt=$(md5sum < $O/$tag.vtm.yuv | cut -c1-32); rm -f $O/$tag.vtm.yuv
$V/prefix/bin/ffmpeg -nostdin -v error -i $bs -f rawvideo -pix_fmt yuv420p10le -y $O/$tag.ff.yuv; ff=$(md5sum < $O/$tag.ff.yuv | cut -c1-32); rm -f $O/$tag.ff.yuv
ur=-; [ -f $O/$tag.rec.yuv ] && { ur=$(md5sum < $O/$tag.rec.yuv | cut -c1-32); rm -f $O/$tag.rec.yuv; }
psnr=$(/usr/bin/ffmpeg -nostdin -v info -f rawvideo -pix_fmt yuv420p10le -s $res -i $O/$tag.dec.yuv -f rawvideo -pix_fmt yuv420p10le -s $res -i $src -lavfi "[0:v][1:v]psnr" -frames:v $nf -f null - 2>&1 | grep -o 'y:[0-9.inf]*' | tail -1 | cut -d: -f2)
rm -f $O/$tag.dec.yuv
echo "$seq,$enc,$pre,$qp,$nf,$es,$fps,$kbps,$psnr,$vd,$vt,$ff,$ur"
