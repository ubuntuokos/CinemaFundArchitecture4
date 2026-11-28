// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors
//
// Test oracle: renders the scene of crates/hrtf/tests/maul_oracle.rs with Maul Audio's C library
// (MIT, https://github.com/Rawframe-Project/maul-audio, commit aeb3834). Never linked or shipped.
// Build from maul-audio/src (clang 18+ or gcc 14+):
//   clang -std=c2x -O2 -I../include -I. maul_harness.c hrtf.c hrtf_file.c file_fields.c crc32.c \
//     hrtf_lookup.c hrtf_resample.c binaural.c binaural_dsp.c near_field.c near_field_table.c \
//     allocator.c fpenv.c -lm -o harness
// Run: harness sadie2-ku100-48k.maudhrtf out.f32 RATE NEAR_FIELD(0|1) DISTANCE_M
// The fixtures in ../data keep the first 4096 stereo frames (interleaved little-endian f32).

#include "maul-audio/hrtf.h"
#include "maul-audio/binaural.h"
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
int main(int argc, char** argv) {
    FILE* f = fopen(argv[1], "rb"); fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
    void* bytes = malloc(n); fread(bytes, 1, n, f); fclose(f);
    maudHrtfDef hd = maudDefaultHrtfDef(); hd.bytes = bytes; hd.byteCount = n; hd.sampleRate = (uint32_t)atoi(argv[3]);
    maudHrtf* h; if (maudLoadHrtf(&hd, &h) != maud_success) { fprintf(stderr, "load\n"); return 1; }
    maudBinauralDef bd = maudDefaultBinauralDef(); bd.hrtf = h; bd.enableNearField = atoi(argv[4]);
    maudBinaural* b; if (maudCreateBinaural(&bd, &b) != maud_success) { fprintf(stderr, "create\n"); return 1; }
    const uint32_t N = 48000, B = 64; float* x = malloc(N*4); float* l = malloc(N*4); float* r = malloc(N*4);
    uint64_t s = 0x12345678ull;
    for (uint32_t i = 0; i < N; ++i) { s ^= s << 13; s ^= s >> 7; s ^= s << 17; x[i] = (float)(s >> 40) / (float)(1ull << 24) - 0.5f; }
    for (uint32_t i = 0; i < N; i += B) {
        float az = 2.0f * 3.14159265f * (float)i / 24000.0f, el = 0.3f * sinf((float)i / 5000.0f), d = (float)atof(argv[5]);
        maudBinauralParams p = { .position = { -sinf(az)*cosf(el)*d, sinf(el)*d, -cosf(az)*cosf(el)*d }, .gain = 0.8f };
        float* outs[2] = { l + i, r + i };
        if (maudBinaural_Process(b, &p, x + i, outs, B) != maud_success) { fprintf(stderr, "process\n"); return 1; }
    }
    FILE* o = fopen(argv[2], "wb"); for (uint32_t i = 0; i < N; ++i) { fwrite(l+i,4,1,o); fwrite(r+i,4,1,o); } fclose(o);
    return 0;
}
