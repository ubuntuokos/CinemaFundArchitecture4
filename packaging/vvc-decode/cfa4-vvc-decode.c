/*
 * cfa4-vvc-decode: the out-of-process H.266/VVC decoder helper of CFA4 (ADR 0013).
 *
 * It wraps VVdeC (BSD-3-Clause-Clear, built from a pinned release by scripts/dev/vvc-decoder.py)
 * and speaks the CFA4 external decoder frame protocol v1 on stdin/stdout, the same protocol as
 * cfa4-av2-decode with its own hello tag. The engine side is the pure-Rust adapter in
 * crates/codecs/src/external_decoder.rs, which documents the protocol. A crash, hang or bad
 * output of this program is an error for the engine, never a crash of CFA4.
 *
 *   cfa4-vvc-decode [--threads N]          protocol mode (stdin requests, stdout replies)
 *   cfa4-vvc-decode --version              prints the VVdeC version and exits
 *
 * Every integer is little-endian. Messages are TAG[4] LEN[u32] PAYLOAD[LEN].
 *   startup  -> "CFA4VVCD" u32 version(1) u32 n  version string (n bytes)
 *   "DATA"   <- i64 pts, then one access unit (Annex B NAL units with start codes)
 *   "FLSH"   <- drain every held picture (end of stream)
 *   "RSET"   <- forget all state (before a seek)
 *   replies  -> zero or more "FRME", at most one "ERRO" (UTF-8 message), then "DONE"
 *   "FRME"   -> u32 w, u32 h, u8 layout (0 4:0:0, 1 4:2:0, 2 4:2:2, 3 4:4:4), u8 bit depth,
 *               u8 matrix, u8 transfer, u8 primaries, u8 full range, u8 colour present,
 *               u8 structure (0 frame, 1 top field, 2 bottom field; a field_seq_flag stream gives one
 *               picture per field, the engine side weaves the pairs), i64 pts, then the planes (Y, U, V; Y only for 4:0:0) without padding,
 *               1 byte per sample at 8 bits, otherwise 2 bytes little-endian.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <vvdec/vvdec.h>

#define PROTOCOL_VERSION 1u
/* Largest request accepted on stdin (one access unit). */
#define MAX_REQUEST (256u * 1024u * 1024u)

static int read_exact(void *buf, size_t n) {
    return fread(buf, 1, n, stdin) == n ? 0 : -1;
}

static int write_exact(const void *buf, size_t n) {
    return fwrite(buf, 1, n, stdout) == n ? 0 : -1;
}

static void put_u32(uint8_t *p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = (uint8_t)(v >> (8 * i));
}

static void put_i64(uint8_t *p, int64_t v) {
    uint64_t u = (uint64_t)v;
    for (int i = 0; i < 8; i++) p[i] = (uint8_t)(u >> (8 * i));
}

static uint32_t get_u32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

static int64_t get_i64(const uint8_t *p) {
    uint64_t u = 0;
    for (int i = 0; i < 8; i++) u |= (uint64_t)p[i] << (8 * i);
    return (int64_t)u;
}

static int write_msg_header(const char tag[4], uint32_t len) {
    uint8_t h[8];
    memcpy(h, tag, 4);
    put_u32(h + 4, len);
    return write_exact(h, sizeof h);
}

static int write_error(const char *msg) {
    size_t n = strlen(msg);
    return write_msg_header("ERRO", (uint32_t)n) || write_exact(msg, n);
}

/* One FRME message: the output picture (conformance window applied by VVdeC), planes packed
 * without stride padding. */
static int write_picture(const vvdecFrame *f) {
    const uint32_t w = f->width, h = f->height, bpc = f->bitDepth;
    const int layout = (int)f->colorFormat;
    if (w == 0 || h == 0 || (bpc != 8 && bpc != 10 && bpc != 12) || layout < 0 || layout > 3)
        return write_error("picture with unsupported parameters");
    const uint32_t planes = layout == VVDEC_CF_YUV400_PLANAR ? 1 : 3;
    if (f->numPlanes < planes) return write_error("picture with missing planes");
    const uint32_t bps = bpc > 8 ? 2 : 1;
    const uint32_t ss_x = layout == VVDEC_CF_YUV420_PLANAR || layout == VVDEC_CF_YUV422_PLANAR;
    const uint32_t ss_y = layout == VVDEC_CF_YUV420_PLANAR;
    const uint32_t cw = (w + ss_x) >> ss_x, ch = (h + ss_y) >> ss_y;
    for (uint32_t pl = 0; pl < planes; pl++) {
        const vvdecPlane *p = &f->planes[pl];
        const uint32_t pw = pl ? cw : w, ph = pl ? ch : h;
        if (!p->ptr || p->width < pw || p->height < ph || (p->bytesPerSample != 1 && p->bytesPerSample != 2) ||
            (p->bytesPerSample == 1 && bps == 2) || p->stride < pw * p->bytesPerSample)
            return write_error("picture plane does not match its size");
    }
    uint64_t size = (uint64_t)w * h * bps;
    if (planes == 3) size += 2 * (uint64_t)cw * ch * bps;
    if (size + 24 > UINT32_MAX) return write_error("picture too large for the frame protocol");

    uint8_t hdr[24];
    put_u32(hdr, w);
    put_u32(hdr + 4, h);
    hdr[8] = (uint8_t)layout;
    hdr[9] = (uint8_t)bpc;
    const vvdecVui *vui = f->picAttributes ? f->picAttributes->vui : NULL;
    const int colour = vui && vui->colourDescriptionPresentFlag;
    hdr[10] = colour ? (uint8_t)vui->matrixCoefficients : 2;
    hdr[11] = colour ? (uint8_t)vui->transferCharacteristics : 2;
    hdr[12] = colour ? (uint8_t)vui->colourPrimaries : 2;
    hdr[13] = vui && vui->videoSignalTypePresentFlag && vui->videoFullRangeFlag;
    hdr[14] = (uint8_t)colour;
    hdr[15] = f->frameFormat == VVDEC_FF_TOP_FIELD ? 1 : f->frameFormat == VVDEC_FF_BOT_FIELD ? 2 : 0;
    put_i64(hdr + 16, f->ctsValid ? (int64_t)f->cts : 0);
    if (write_msg_header("FRME", (uint32_t)(size + sizeof hdr)) || write_exact(hdr, sizeof hdr)) return -1;
    for (uint32_t pl = 0; pl < planes; pl++) {
        const vvdecPlane *p = &f->planes[pl];
        const uint32_t pw = pl ? cw : w, ph = pl ? ch : h;
        const unsigned char *row = p->ptr;
        for (uint32_t y = 0; y < ph; y++, row += p->stride) {
            if (p->bytesPerSample == bps && bps == 1) {
                if (write_exact(row, pw)) return -1;
            } else if (p->bytesPerSample == 1) {
                return -1; /* excluded above */
            } else {
                /* 16-bit samples in host order; little-endian on the wire, one byte at 8 bits */
                const uint16_t *s = (const uint16_t *)(const void *)row;
                for (uint32_t x = 0; x < pw; x++) {
                    int r;
                    if (bps == 1) {
                        uint8_t b = (uint8_t)s[x];
                        r = write_exact(&b, 1);
                    } else {
                        uint8_t b[2] = { (uint8_t)s[x], (uint8_t)(s[x] >> 8) };
                        r = write_exact(b, 2);
                    }
                    if (r) return -1;
                }
            }
        }
    }
    return 0;
}

typedef struct {
    vvdecDecoder *dec;
    vvdecAccessUnit *au;
    int threads;
} Ctx;

static int open_decoder(Ctx *c) {
    vvdecParams p;
    vvdec_params_default(&p);
    p.threads = c->threads > 0 ? c->threads : -1;
    p.logLevel = VVDEC_SILENT;
    p.errHandlingFlags = VVDEC_ERR_HANDLING_TRY_CONTINUE;
    c->dec = vvdec_decoder_open(&p);
    return c->dec ? 0 : -1;
}

static void close_decoder(Ctx *c) {
    if (c->dec) vvdec_decoder_close(c->dec);
    c->dec = NULL;
}

static void describe(Ctx *c, int res, const char *what, char *err, size_t errlen) {
    const char *last = c->dec ? vvdec_get_last_error(c->dec) : NULL;
    snprintf(err, errlen, "%s failed: %s%s%s", what, vvdec_get_error_msg(res), last && *last ? ": " : "", last ? last : "");
}

/* Hand over one returned frame. Returns <0 on an I/O error. */
static int emit(Ctx *c, vvdecFrame *f) {
    const int w = write_picture(f);
    vvdec_frame_unref(c->dec, f);
    return w;
}

/* Feed one access unit, then collect every picture VVdeC has ready (an empty access unit returns
 * pending pictures). Returns <0 on an I/O error, 1 on a decode error. */
static int decode_unit(Ctx *c, const uint8_t *buf, size_t n, int64_t pts, char *err, size_t errlen) {
    if (n > (size_t)c->au->payloadSize) {
        vvdec_accessUnit_free_payload(c->au);
        vvdec_accessUnit_alloc_payload(c->au, (int)n);
        if (!c->au->payload) {
            snprintf(err, errlen, "out of memory");
            return 1;
        }
    }
    memcpy(c->au->payload, buf, n);
    c->au->payloadUsedSize = (int)n;
    c->au->cts = (uint64_t)pts;
    c->au->ctsValid = true;
    c->au->dtsValid = false;
    for (int round = 0;; round++) {
        vvdecFrame *f = NULL;
        const int res = vvdec_decode(c->dec, c->au, &f);
        c->au->payloadUsedSize = 0;
        if (f && emit(c, f)) return -1;
        if (res == VVDEC_TRY_AGAIN || res == VVDEC_EOF || (res == VVDEC_OK && !f)) return 0;
        if (res != VVDEC_OK) {
            describe(c, res, "vvdec_decode", err, errlen);
            return 1;
        }
        if (round > 64) return 0; /* bounded: pictures left over come with the next request */
    }
}

/* End of stream: hand over every held picture, then reopen so the decoder accepts data again. */
static int flush_all(Ctx *c, char *err, size_t errlen) {
    int status = 0;
    for (;;) {
        vvdecFrame *f = NULL;
        const int res = vvdec_flush(c->dec, &f);
        if (f && emit(c, f)) return -1;
        if (res == VVDEC_EOF || !f) break;
        if (res != VVDEC_OK) {
            describe(c, res, "vvdec_flush", err, errlen);
            status = 1;
            break;
        }
    }
    close_decoder(c);
    if (open_decoder(c)) {
        snprintf(err, errlen, "vvdec_decoder_open failed after a flush");
        return 1;
    }
    return status;
}

static int run(int threads) {
    Ctx c = { NULL, NULL, threads };
    c.au = vvdec_accessUnit_alloc();
    if (!c.au || open_decoder(&c)) {
        fprintf(stderr, "cfa4-vvc-decode: vvdec_decoder_open failed\n");
        if (c.au) vvdec_accessUnit_free(c.au);
        return 2;
    }
    vvdec_accessUnit_alloc_payload(c.au, 1 << 20);
    const char *ver = vvdec_get_version();
    uint8_t hello[16];
    memcpy(hello, "CFA4VVCD", 8);
    put_u32(hello + 8, PROTOCOL_VERSION);
    put_u32(hello + 12, (uint32_t)strlen(ver));
    if (!c.au->payload || write_exact(hello, sizeof hello) || write_exact(ver, strlen(ver)) || fflush(stdout)) {
        close_decoder(&c);
        vvdec_accessUnit_free(c.au);
        return 2;
    }

    uint8_t *buf = NULL;
    int status = 0;
    for (;;) {
        uint8_t h[8];
        if (read_exact(h, sizeof h)) break; /* end of input: normal exit */
        const uint32_t len = get_u32(h + 4);
        if (len > MAX_REQUEST) {
            fprintf(stderr, "cfa4-vvc-decode: request of %" PRIu32 " bytes refused\n", len);
            status = 2;
            break;
        }
        uint8_t *nb = realloc(buf, len ? len : 1);
        if (!nb) {
            status = 2;
            break;
        }
        buf = nb;
        if (len && read_exact(buf, len)) {
            status = 2;
            break;
        }
        char err[200] = "";
        int r = 0;
        if (!memcmp(h, "DATA", 4)) {
            if (len < 8) {
                snprintf(err, sizeof err, "DATA request without a pts");
                r = 1;
            } else if (len > 8) {
                r = decode_unit(&c, buf + 8, len - 8, get_i64(buf), err, sizeof err);
            }
        } else if (!memcmp(h, "FLSH", 4)) {
            r = flush_all(&c, err, sizeof err);
        } else if (!memcmp(h, "RSET", 4)) {
            close_decoder(&c);
            if (open_decoder(&c)) {
                snprintf(err, sizeof err, "vvdec_decoder_open failed after a reset");
                r = 1;
            }
        } else {
            fprintf(stderr, "cfa4-vvc-decode: unknown request\n");
            status = 2;
            break;
        }
        if (r < 0 || (r > 0 && write_error(err)) || write_msg_header("DONE", 0) || fflush(stdout)) {
            status = 2;
            break;
        }
        if (!c.dec) {
            status = 2;
            break;
        }
    }
    free(buf);
    close_decoder(&c);
    vvdec_accessUnit_free(c.au);
    return status;
}

int main(int argc, char **argv) {
    int threads = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--version")) {
            printf("%s\n", vvdec_get_version());
            return 0;
        } else if (!strcmp(argv[i], "--threads") && i + 1 < argc) {
            threads = atoi(argv[++i]);
            if (threads < 0 || threads > 256) threads = 0;
        } else {
            fprintf(stderr, "usage: cfa4-vvc-decode [--threads N] | --version\n");
            return 2;
        }
    }
    return run(threads);
}
