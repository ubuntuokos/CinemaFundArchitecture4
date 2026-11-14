/*
 * The G.729 test oracle of CFA4 (crates/g729): encodes and decodes G.729 / Annex B with bcg729
 * (GPL-3.0-or-later), on raw 16-bit little-endian mono 8 kHz PCM and ITU-T serial bitstreams
 * (`.bit`: per frame a sync word 0x6b21, or 0x6b20 for an erased frame, a bit count (80 speech,
 * 16 SID, 0 not sent) and one 16-bit word per bit, 0x0081 for 1 and 0x007f for 0). It is an
 * external test tool, never linked into or shipped with CFA4 (scripts/ci/install-g729-oracle.sh
 * builds it).
 *
 *   cfa4-g729-oracle enc VAD in.raw out.bit   VAD 0 or 1 (Annex B DTX: SID and untransmitted frames)
 *   cfa4-g729-oracle dec in.bit out.raw
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <bcg729/decoder.h>
#include <bcg729/encoder.h>

static int usage(void) {
    fprintf(stderr, "usage: cfa4-g729-oracle enc VAD in.raw out.bit | dec in.bit out.raw\n");
    return 2;
}

static void put16(FILE *f, unsigned v) {
    fputc(v & 0xff, f);
    fputc(v >> 8, f);
}

static int get16(FILE *f, unsigned *v) {
    int a = fgetc(f), b = fgetc(f);
    if (a == EOF || b == EOF)
        return 0;
    *v = (unsigned)a | ((unsigned)b << 8);
    return 1;
}

static int encode(int vad, const char *in, const char *out) {
    FILE *fi = fopen(in, "rb"), *fo = fopen(out, "wb");
    if (!fi || !fo) {
        perror("open");
        return 1;
    }
    bcg729EncoderChannelContextStruct *enc = initBcg729EncoderChannel(vad ? 1 : 0);
    int16_t pcm[80];
    uint8_t bits[10];
    while (fread(pcm, 2, 80, fi) == 80) {
        uint8_t len = 0;
        bcg729Encoder(enc, pcm, bits, &len);
        unsigned nbits = len == 10 ? 80 : len == 2 ? 16 : 0;
        put16(fo, 0x6b21);
        put16(fo, nbits);
        for (unsigned k = 0; k < nbits; k++)
            put16(fo, (bits[k / 8] >> (7 - k % 8)) & 1 ? 0x0081 : 0x007f);
    }
    closeBcg729EncoderChannel(enc);
    fclose(fi);
    return fclose(fo) == 0 ? 0 : 1;
}

static int decode(const char *in, const char *out) {
    FILE *fi = fopen(in, "rb"), *fo = fopen(out, "wb");
    if (!fi || !fo) {
        perror("open");
        return 1;
    }
    bcg729DecoderChannelContextStruct *dec = initBcg729DecoderChannel();
    unsigned sync, nbits;
    while (get16(fi, &sync) && get16(fi, &nbits)) {
        if (sync != 0x6b21 && sync != 0x6b20)
            break;
        uint8_t bits[10];
        memset(bits, 0, sizeof bits);
        unsigned k;
        for (k = 0; k < nbits; k++) {
            unsigned w;
            if (!get16(fi, &w))
                break;
            if (k < 80 && w == 0x0081)
                bits[k / 8] |= 0x80 >> (k % 8);
        }
        if (k < nbits)
            break;
        int16_t pcm[80];
        if (sync == 0x6b21 && nbits == 80)
            bcg729Decoder(dec, bits, 10, 0, 0, 0, pcm);
        else if (sync == 0x6b21 && (nbits == 15 || nbits == 16))
            bcg729Decoder(dec, bits, 2, 0, 1, 0, pcm);
        else
            bcg729Decoder(dec, NULL, 0, 1, 0, 0, pcm);
        if (fwrite(pcm, 2, 80, fo) != 80)
            return 1;
    }
    closeBcg729DecoderChannel(dec);
    fclose(fi);
    return fclose(fo) == 0 ? 0 : 1;
}

int main(int argc, char **argv) {
    if (argc == 5 && strcmp(argv[1], "enc") == 0)
        return encode(atoi(argv[2]), argv[3], argv[4]);
    if (argc == 4 && strcmp(argv[1], "dec") == 0)
        return decode(argv[2], argv[3]);
    return usage();
}
