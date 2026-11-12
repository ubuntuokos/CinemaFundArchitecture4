/*
 * cfa4-av2-decode: the out-of-process AV2 decoder helper of CFA4 (ADR 0010, A track).
 *
 * It wraps dav2d (BSD-2-Clause, built from a pinned commit by scripts/dev/av2-decoder.py) and
 * speaks the CFA4 AV2 frame protocol v1 on stdin/stdout. The engine side is the pure-Rust
 * adapter in crates/codecs/src/av2_external.rs, which documents the protocol. A crash, hang or
 * bad output of this program is an error for the engine, never a crash of CFA4.
 *
 *   cfa4-av2-decode [--threads N]          protocol mode (stdin requests, stdout replies)
 *   cfa4-av2-decode --version              prints the dav2d version and exits
 *
 * Every integer is little-endian. Messages are TAG[4] LEN[u32] PAYLOAD[LEN].
 *   startup  -> "CFA4AV2D" u32 version(1) u32 n  version string (n bytes)
 *   "DATA"   <- i64 pts, then one temporal unit (Annex B OBUs, starting with the temporal delimiter)
 *   "FLSH"   <- drain every held picture (end of stream)
 *   "RSET"   <- forget all state (before a seek)
 *   replies  -> zero or more "FRME", at most one "ERRO" (UTF-8 message), then "DONE"
 *   "FRME"   -> u32 w, u32 h, u8 layout (0 4:0:0, 1 4:2:0, 2 4:2:2, 3 4:4:4), u8 bit depth,
 *               u8 matrix, u8 transfer, u8 primaries, u8 full range, u8 colour present, u8 0,
 *               i64 pts, then the planes (Y, U, V; Y only for 4:0:0) without padding,
 *               1 byte per sample at 8 bits, otherwise 2 bytes little-endian.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <errno.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <dav2d/dav2d.h>

#define PROTOCOL_VERSION 1u
/* Largest request accepted on stdin (one temporal unit). */
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

/* One FRME message: the visible area of the picture, planes packed without stride padding. */
static int write_picture(const Dav2dPicture *p) {
    const int w = p->p.w, h = p->p.h, bpc = p->p.bpc;
    const int layout = (int)p->p.layout;
    if (w <= 0 || h <= 0 || (bpc != 8 && bpc != 10 && bpc != 12) || layout < 0 || layout > 3)
        return write_error("picture with unsupported parameters");
    const int bps = bpc > 8 ? 2 : 1;
    const int ss_x = layout == DAV2D_PIXEL_LAYOUT_I420 || layout == DAV2D_PIXEL_LAYOUT_I422;
    const int ss_y = layout == DAV2D_PIXEL_LAYOUT_I420;
    const int cw = (w + ss_x) >> ss_x, ch = (h + ss_y) >> ss_y;
    const int planes = layout == DAV2D_PIXEL_LAYOUT_I400 ? 1 : 3;
    uint64_t size = (uint64_t)w * h * bps;
    if (planes == 3) size += 2 * (uint64_t)cw * ch * bps;
    if (size + 24 > UINT32_MAX) return write_error("picture too large for the frame protocol");

    uint8_t hdr[24];
    put_u32(hdr, (uint32_t)w);
    put_u32(hdr + 4, (uint32_t)h);
    hdr[8] = (uint8_t)layout;
    hdr[9] = (uint8_t)bpc;
    const Dav2dContentInterpretation *ci = p->ci;
    const int colour = ci && ci->color_description_present;
    hdr[10] = colour ? ci->color.mtrx : 2;
    hdr[11] = colour ? ci->color.trc : 2;
    hdr[12] = colour ? ci->color.pri : 2;
    hdr[13] = colour ? ci->color.range : 0;
    hdr[14] = (uint8_t)colour;
    hdr[15] = 0;
    put_i64(hdr + 16, p->m.timestamp);
    if (write_msg_header("FRME", (uint32_t)(size + sizeof hdr)) || write_exact(hdr, sizeof hdr)) return -1;
    for (int pl = 0; pl < planes; pl++) {
        const int pw = pl ? cw : w, ph = pl ? ch : h;
        const ptrdiff_t stride = p->stride[pl ? 1 : 0];
        const uint8_t *row = p->data[pl];
        for (int y = 0; y < ph; y++, row += stride) {
            if (bps == 1) {
                if (write_exact(row, (size_t)pw)) return -1;
            } else {
                /* little-endian on disk whatever the host order */
                const uint16_t *s = (const uint16_t *)row;
                for (int x = 0; x < pw; x++) {
                    uint8_t b[2] = { (uint8_t)s[x], (uint8_t)(s[x] >> 8) };
                    if (write_exact(b, 2)) return -1;
                }
            }
        }
    }
    return 0;
}

/* Pull every picture the decoder has ready (all of them after dav2d_send_data(c, NULL)).
 * Returns <0 on an I/O error, 1 on a decode error. */
static int drain_ready(Dav2dContext *c, char *err, size_t errlen) {
    for (;;) {
        Dav2dPicture pic = { 0 };
        const int res = dav2d_get_picture(c, &pic);
        if (res == DAV2D_ERR(EAGAIN) || res == DAV2D_EOF) return 0;
        if (res < 0) {
            snprintf(err, errlen, "dav2d_get_picture failed (%d)", res);
            return 1;
        }
        const int w = write_picture(&pic);
        dav2d_picture_unref(&pic);
        if (w) return -1;
    }
}

static int decode_unit(Dav2dContext *c, const uint8_t *buf, size_t n, int64_t pts, char *err, size_t errlen) {
    Dav2dData data = { 0 };
    uint8_t *dst = dav2d_data_create(&data, n);
    if (!dst) {
        snprintf(err, errlen, "out of memory");
        return 1;
    }
    memcpy(dst, buf, n);
    data.m.timestamp = pts;
    do {
        const int res = dav2d_send_data(c, &data);
        if (res < 0 && res != DAV2D_ERR(EAGAIN)) {
            dav2d_data_unref(&data);
            snprintf(err, errlen, "dav2d_send_data failed (%d)", res);
            return 1;
        }
        const int d = drain_ready(c, err, errlen);
        if (d) {
            dav2d_data_unref(&data);
            return d;
        }
    } while (data.sz > 0);
    return 0;
}

static int run(int threads) {
    Dav2dSettings s;
    dav2d_default_settings(&s);
    s.n_threads = threads;
    Dav2dContext *c = NULL;
    if (dav2d_open(&c, &s) < 0) {
        fprintf(stderr, "cfa4-av2-decode: dav2d_open failed\n");
        return 2;
    }
    const char *ver = dav2d_version();
    uint8_t hello[16];
    memcpy(hello, "CFA4AV2D", 8);
    put_u32(hello + 8, PROTOCOL_VERSION);
    put_u32(hello + 12, (uint32_t)strlen(ver));
    if (write_exact(hello, sizeof hello) || write_exact(ver, strlen(ver)) || fflush(stdout)) {
        dav2d_close(&c);
        return 2;
    }

    uint8_t *buf = NULL;
    int status = 0;
    for (;;) {
        uint8_t h[8];
        if (read_exact(h, sizeof h)) break; /* end of input: normal exit */
        const uint32_t len = get_u32(h + 4);
        if (len > MAX_REQUEST) {
            fprintf(stderr, "cfa4-av2-decode: request of %" PRIu32 " bytes refused\n", len);
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
        char err[160] = "";
        int r = 0;
        if (!memcmp(h, "DATA", 4)) {
            if (len < 8) {
                snprintf(err, sizeof err, "DATA request without a pts");
                r = 1;
            } else if (len > 8) {
                r = decode_unit(c, buf + 8, len - 8, get_i64(buf), err, sizeof err);
            }
        } else if (!memcmp(h, "FLSH", 4)) {
            /* end of stream: hand over the held pictures; the decoder accepts data again after EOF */
            dav2d_send_data(c, NULL);
            r = drain_ready(c, err, sizeof err);
        } else if (!memcmp(h, "RSET", 4)) {
            dav2d_flush(c);
        } else {
            fprintf(stderr, "cfa4-av2-decode: unknown request\n");
            status = 2;
            break;
        }
        if (r < 0 || (r > 0 && write_error(err)) || write_msg_header("DONE", 0) || fflush(stdout)) {
            status = 2;
            break;
        }
    }
    free(buf);
    dav2d_close(&c);
    return status;
}

int main(int argc, char **argv) {
    int threads = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--version")) {
            printf("%s\n", dav2d_version());
            return 0;
        } else if (!strcmp(argv[i], "--threads") && i + 1 < argc) {
            threads = atoi(argv[++i]);
            if (threads < 0 || threads > 256) threads = 0;
        } else {
            fprintf(stderr, "usage: cfa4-av2-decode [--threads N] | --version\n");
            return 2;
        }
    }
    return run(threads);
}
