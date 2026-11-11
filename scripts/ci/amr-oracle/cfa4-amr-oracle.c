/*
 * The AMR test oracle of CFA4 (crates/amr): encodes and decodes AMR-NB with opencore-amrnb and
 * AMR-WB with vo-amrwbenc / opencore-amrwb (Apache-2.0), on raw 16-bit little-endian mono PCM
 * and RFC 4867 storage files (`#!AMR\n`, `#!AMR-WB\n`). It is an external test tool, never
 * linked into or shipped with CFA4 (scripts/ci/install-amr-oracle.sh builds it).
 *
 *   cfa4-amr-oracle enc-nb MODE DTX in.raw out.amr   MODE 0-7, or "cycle" (mode changes per frame)
 *   cfa4-amr-oracle enc-wb MODE DTX in.raw out.awb   MODE 0-8, or "cycle"
 *   cfa4-amr-oracle dec-nb in.amr out.raw
 *   cfa4-amr-oracle dec-wb in.awb out.raw
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <opencore-amrnb/interf_dec.h>
#include <opencore-amrnb/interf_enc.h>
#include <opencore-amrwb/dec_if.h>
#include <vo-amrwbenc/enc_if.h>

/* Storage frame sizes including the header byte, by frame type (RFC 4867 §5.3). */
static const int nb_size[16] = {13, 14, 16, 18, 20, 21, 27, 32, 6, 1, 1, 1, 1, 1, 1, 1};
static const int wb_size[16] = {18, 24, 33, 37, 41, 47, 51, 59, 61, 6, 1, 1, 1, 1, 1, 1};

static int usage(void) {
    fprintf(stderr, "usage: cfa4-amr-oracle enc-nb|enc-wb MODE DTX in.raw out | dec-nb|dec-wb in out.raw\n");
    return 2;
}

static int encode(int wb, const char *mode_arg, int dtx, const char *in, const char *out) {
    int cycle = strcmp(mode_arg, "cycle") == 0;
    int mode = cycle ? 0 : atoi(mode_arg);
    int modes = wb ? 9 : 8, n = wb ? 320 : 160;
    if (mode < 0 || mode >= modes)
        return usage();
    FILE *fi = fopen(in, "rb"), *fo = fopen(out, "wb");
    if (!fi || !fo) {
        perror("open");
        return 1;
    }
    void *enc = wb ? E_IF_init() : Encoder_Interface_init(dtx);
    fputs(wb ? "#!AMR-WB\n" : "#!AMR\n", fo);
    short pcm[320];
    unsigned char buf[64];
    for (int frame = 0;; frame++) {
        size_t got = fread(pcm, 2, n, fi);
        if (got == 0)
            break;
        memset(pcm + got, 0, (n - got) * 2);
        if (cycle)
            mode = (frame / 3) % modes;
        int len = wb ? E_IF_encode(enc, mode, pcm, buf, dtx) : Encoder_Interface_Encode(enc, (enum Mode)mode, pcm, buf, 0);
        if (len <= 0)
            return 1;
        fwrite(buf, 1, len, fo);
    }
    if (wb)
        E_IF_exit(enc);
    else
        Encoder_Interface_exit(enc);
    fclose(fi);
    return fclose(fo) != 0;
}

static int decode(int wb, const char *in, const char *out) {
    FILE *fi = fopen(in, "rb"), *fo = fopen(out, "wb");
    if (!fi || !fo) {
        perror("open");
        return 1;
    }
    const char *magic = wb ? "#!AMR-WB\n" : "#!AMR\n";
    char head[16];
    size_t mlen = strlen(magic);
    if (fread(head, 1, mlen, fi) != mlen || memcmp(head, magic, mlen) != 0) {
        fprintf(stderr, "not an %s storage file\n", wb ? "AMR-WB" : "AMR");
        return 1;
    }
    void *dec = wb ? D_IF_init() : Decoder_Interface_init();
    const int *size = wb ? wb_size : nb_size;
    int n = wb ? 320 : 160;
    unsigned char buf[64];
    short pcm[320];
    while (fread(buf, 1, 1, fi) == 1) {
        int len = size[(buf[0] >> 3) & 15];
        if (len > 1 && fread(buf + 1, 1, len - 1, fi) != (size_t)(len - 1))
            break;
        /* Frame types the decoder skips leave the buffer untouched: write silence for them. */
        memset(pcm, 0, sizeof pcm);
        if (wb)
            D_IF_decode(dec, buf, pcm, 0);
        else
            Decoder_Interface_Decode(dec, buf, pcm, 0);
        fwrite(pcm, 2, n, fo);
    }
    if (wb)
        D_IF_exit(dec);
    else
        Decoder_Interface_exit(dec);
    fclose(fi);
    return fclose(fo) != 0;
}

int main(int argc, char **argv) {
    if (argc == 6 && (!strcmp(argv[1], "enc-nb") || !strcmp(argv[1], "enc-wb")))
        return encode(argv[1][4] == 'w', argv[2], atoi(argv[3]), argv[4], argv[5]);
    if (argc == 4 && (!strcmp(argv[1], "dec-nb") || !strcmp(argv[1], "dec-wb")))
        return decode(argv[1][4] == 'w', argv[2], argv[3]);
    return usage();
}
