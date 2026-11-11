#!/usr/bin/env bash
# Seed corpus for the CFA4 fuzz targets (CFA4-SECURITY-001 B2): small valid samples generated with
# ffmpeg into fuzz/corpus/<target>/ (gitignored; media is never committed, docs/testing.md).
# Each target's seeds use the byte layout its harness expects (fuzz/src/lib.rs): raw streams for the
# elementary-stream decoders, parameter bytes first where a decoder needs them, and 2-byte
# big-endian lengths before each packet for the packet-fed decoders.
# Usage: fuzz/seeds.sh  (needs ffmpeg and python3; formats ffmpeg cannot write are skipped)
set -uo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
C="$HERE/corpus"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
command -v ffmpeg >/dev/null || { echo "ffmpeg not found" >&2; exit 2; }
mkdir -p "$C"

ff() { ffmpeg -hide_banner -loglevel error -y "$@" || echo "skip: ffmpeg $*" >&2; }
V="-f lavfi -i testsrc2=size=64x48:rate=25:duration=0.2"
VA="$V -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2"
A="-f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2"

# containers
ff $VA -c:v libx264 -c:a aac "$T/a.mp4"
ff $VA -c:v libx264 -c:a aac -movflags +faststart+frag_keyframe+empty_moov "$T/frag.mp4"
ff $VA -c:v prores -c:a pcm_s16le "$T/a.mov"
ff $VA -c:v libvpx-vp9 -c:a libopus "$T/a.webm"
ff $VA -c:v ffv1 -c:a flac "$T/a.mkv"
ff $VA -c:v mpeg2video -c:a pcm_s16le -ar 48000 -f mxf "$T/a.mxf"
ff $V -c:v dnxhd -s 1920x1080 -b:v 36M -pix_fmt yuv422p -f mxf_opatom "$T/opatom.mxf"
ff $VA -c:v huffyuv -c:a pcm_s16le "$T/a.avi"
ff $VA -c:v libx264 -c:a mp2 -f mpegts "$T/a.ts"
ff $A -c:a libopus "$T/a.ogg"
ff $V -frames:v 1 -c:v libaom-av1 -still-picture 1 "$T/a.avif"
ff -f lavfi -i testsrc2=size=720x576:rate=25:duration=0.08 -c:v dvvideo -pix_fmt yuv420p -f dv "$T/a.dv"
mkdir -p "$C/isobmff" "$C/matroska" "$C/mxf" "$C/avi" "$C/mpegts" "$C/ogg" "$C/heif" "$C/dv"
cp "$T/a.mp4" "$T/frag.mp4" "$T/a.mov" "$C/isobmff/" 2>/dev/null
cp "$T/a.webm" "$T/a.mkv" "$C/matroska/" 2>/dev/null
cp "$T/a.mxf" "$T/opatom.mxf" "$C/mxf/" 2>/dev/null
cp "$T/a.avi" "$C/avi/" 2>/dev/null
cp "$T/a.ts" "$C/mpegts/" 2>/dev/null
cp "$T/a.ogg" "$C/ogg/" 2>/dev/null
cp "$T/a.avif" "$C/heif/" 2>/dev/null
cp "$T/a.dv" "$C/dv/" 2>/dev/null

# elementary streams and single frames
ff $V -c:v libx264 -bsf:v h264_mp4toannexb -f h264 "$T/a.h264"
ff $V -c:v libx264 -profile:v high -x264-params cabac=1:bframes=2 -f h264 "$T/b.h264"
ff $V -c:v libx265 -x265-params log-level=error -f hevc "$T/a.hevc"
ff $V -c:v mpeg2video -f mpeg2video "$T/a.m2v"
ff $V -c:v libaom-av1 -cpu-used 8 -f obu "$T/a.obu"
ff $V -c:v libvpx -f ivf "$T/vp8.ivf"
ff $V -c:v libvpx-vp9 -f ivf "$T/vp9.ivf"
ff $V -frames:v 1 -c:v prores_ks -f rawvideo "$T/a.prores"
ff $V -frames:v 1 -s 1920x1080 -c:v dnxhd -b:v 36M -pix_fmt yuv422p -f rawvideo "$T/a.dnx"
ff $V -frames:v 1 -c:v cfhd -f rawvideo "$T/a.cfhd"
ff $V -frames:v 1 -c:v libopenjpeg "$T/a.jp2"
ff $V -frames:v 1 -c:v libopenjpeg -f rawvideo "$T/a.j2k"
ff $V -frames:v 1 -c:v libjxl "$T/a.jxl"
ff $V -frames:v 1 "$T/a.tiff"
ff $V -frames:v 1 -c:v hap -f rawvideo "$T/a.hap"
ff $V -frames:v 1 -c:v utvideo -f rawvideo "$T/a.utvideo"
ff $V -frames:v 1 -c:v huffyuv -pix_fmt yuv422p -f rawvideo "$T/a.huffyuv"
ff $V -frames:v 1 -c:v ffv1 -level 1 -f rawvideo "$T/a.ffv1"
ff $A -c:a aac -f adts "$T/a.adts"
ff $A -c:a ac3 "$T/a.ac3"
ff $A -c:a mp2 -f mp2 "$T/a.mp2"
ff $A -c:a libmp3lame -f mp3 "$T/a.mp3"
ff $A -ac 2 -c:a dca -strict -2 -f dts "$T/a.dts"
ff $A -c:a truehd -strict -2 -f truehd "$T/a.thd"
ff $A -c:a adpcm_ima_wav "$T/ima.wav"
ff $A -c:a adpcm_ms "$T/ms.wav"
ff $A -ar 16000 -c:a g722 "$T/g722.wav"
ff $A -c:a pcm_mulaw "$T/mulaw.wav"

python3 - "$T" "$C" <<'PY'
import os, struct, sys
T, C = sys.argv[1], sys.argv[2]

def rd(name):
    p = os.path.join(T, name)
    return open(p, "rb").read() if os.path.exists(p) else None

def put(target, name, data):
    if data is None:
        return
    d = os.path.join(C, target)
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, name), "wb").write(data)

def packets(chunks):
    out = b""
    for c in chunks[:64]:
        c = c[:0xFFFF]
        out += struct.pack(">H", len(c)) + c
    return out

def side(n):  # the harness's side(): 1 + u16 % 512
    return struct.pack(">H", n - 1)

def ivf(data):
    if not data or data[:4] != b"DKIF":
        return []
    pos, frames = struct.unpack("<H", data[6:8])[0], []
    while pos + 12 <= len(data):
        n = struct.unpack("<I", data[pos:pos + 4])[0]
        frames.append(data[pos + 12:pos + 12 + n])
        pos += 12 + n
    return frames

def split_sync(data, sync):
    if not data:
        return []
    idx = [i for i in range(len(data) - len(sync)) if data[i:i + len(sync)] == sync]
    return [data[a:b] for a, b in zip(idx, idx[1:] + [len(data)])]

def adts(data):
    frames, pos = [], 0
    while data and pos + 7 <= len(data) and data[pos] == 0xFF:
        n = ((data[pos + 3] & 3) << 11) | (data[pos + 4] << 3) | (data[pos + 5] >> 5)
        if n < 7:
            break
        frames.append(data[pos + 7:pos + n])
        pos += n
    return frames

def wav_data(data):
    if not data or data[:4] != b"RIFF":
        return None, None
    pos, fmt, body = 12, None, None
    while pos + 8 <= len(data):
        cid, n = data[pos:pos + 4], struct.unpack("<I", data[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            fmt = data[pos + 8:pos + 8 + n]
        if cid == b"data":
            body = data[pos + 8:pos + 8 + n]
        pos += 8 + n + (n & 1)
    return fmt, body

def ogg_packets(data):
    pkts, cur, pos = [], b"", 0
    while data and pos + 27 <= len(data) and data[pos:pos + 4] == b"OggS":
        nseg = data[pos + 26]
        lacing = data[pos + 27:pos + 27 + nseg]
        body = pos + 27 + nseg
        for l in lacing:
            cur += data[body:body + l]
            body += l
            if l < 255:
                pkts.append(cur)
                cur = b""
        pos = body
    return pkts

for name in ["a.h264", "b.h264"]:
    put("h264", name, rd(name))
put("hevc", "a.hevc", rd("a.hevc"))
put("mpeg2v", "a.m2v", rd("a.m2v"))
put("av1", "a.obu", rd("a.obu"))
put("lcevc", "h264-annexb", b"\x00\x00" + (rd("a.h264") or b""))
put("vp8", "a", packets(ivf(rd("vp8.ivf"))))
put("vp9", "a", packets(ivf(rd("vp9.ivf"))))
put("prores", "a", rd("a.prores"))
put("dnx", "a", rd("a.dnx"))
put("cineform", "a", rd("a.cfhd"))
put("j2k", "a.jp2", rd("a.jp2"))
put("j2k", "a.j2k", rd("a.j2k"))
put("jxl", "a.jxl", rd("a.jxl"))
put("dng", "a.tiff", rd("a.tiff"))
# hap: fourcc index 0 (Hap1), 64x48
if rd("a.hap"):
    put("hap", "hap1-64x48", b"\x00" + side(64) + side(48) + rd("a.hap"))
# utvideo: fourcc index 3 (ULY2), 64x48, 16 bytes of extradata (zeros: the decoder must cope)
if rd("a.utvideo"):
    put("utvideo", "uly2-64x48", b"\x03" + side(64) + side(48) + bytes(16) + rd("a.utvideo"))
# huffyuv: 64x48, 16 bits per pixel (index 0), no extradata
if rd("a.huffyuv"):
    put("huffyuv", "yuy2-64x48", side(64) + side(48) + b"\x00\x00" + rd("a.huffyuv"))
# ffv1 level 1: no configuration record
if rd("a.ffv1"):
    put("ffv1", "v1-64x48", side(64) + side(48) + b"\x00" + rd("a.ffv1"))
# aac: AudioSpecificConfig of AAC-LC 48 kHz stereo, then the raw frames
put("aac", "lc-48k", b"\x02\x11\x90" + packets(adts(rd("a.adts"))))
put("ac3", "a.ac3", rd("a.ac3"))
put("mpa", "mp2", packets(split_sync(rd("a.mp2"), b"\xff\xfd") or split_sync(rd("a.mp2"), b"\xff\xfc")))
put("mpa", "mp3", packets(split_sync(rd("a.mp3"), b"\xff\xfb")))
put("dts", "core", packets(split_sync(rd("a.dts"), b"\x7f\xfe\x80\x01")))
thd = rd("a.thd")
if thd:
    put("truehd", "a", packets([thd[i:i + 1024] for i in range(0, min(len(thd), 64 * 1024), 1024)]))
ogg = rd("a.ogg")
if ogg:
    put("opus", "stereo", b"\x01" + packets(ogg_packets(ogg)[2:]))
for codec, name in [(3, "ima.wav"), (5, "ms.wav"), (2, "g722.wav"), (0, "mulaw.wav")]:
    fmt, body = wav_data(rd(name))
    if body is None:
        continue
    channels = struct.unpack("<H", fmt[2:4])[0] if fmt else 1
    align = struct.unpack("<H", fmt[12:14])[0] if fmt else 1
    put("adpcm", name, bytes([channels - 1]) + struct.pack(">H", max(align - 1, 0)) + bytes([codec]) + body)
PY

# text formats
mkdir -p "$C/captions" "$C/interchange" "$C/project"
printf '\x00%s' $'1\n00:00:01,000 --> 00:00:02,500\nHello <i>world</i>\n\n2\n00:00:03,000 --> 00:00:04,000\nSecond\n' >"$C/captions/srt"
printf '\x01%s' $'WEBVTT\n\nNOTE x\n\n00:01.000 --> 00:02.000 line:0 position:50%\nHi\n' >"$C/captions/vtt"
printf '\x02%s' $'Scenarist_SCC V1.0\n\n00:00:00:00\t9420 9420 94ae 94ae 9452 9452 97a2 97a2 c8e5 ecec ef80 942c 942c 942f 942f\n' >"$C/captions/scc"
printf '\x00%s' $'TITLE: seed\nFCM: NON-DROP FRAME\n\n001  AX       V     C        00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\n* FROM CLIP NAME: a.mov\n' >"$C/interchange/edl"
echo "corpus: $(find "$C" -type f | wc -l) files in $C"
