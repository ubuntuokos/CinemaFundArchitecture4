/*
 * libFuzzer harness for VVdeC as cfa4-vvc-decode drives it (ADR 0013, VVC-TEST-017).
 *
 * The input is an Annex B byte stream. It is cut into access units before every access unit
 * delimiter NAL unit (type 20; the whole input is one unit when there is none), each unit goes
 * through vvdec_decode() exactly as the helper calls it (error handling TRY_CONTINUE, no logging),
 * every returned picture is touched (all planes read) and released, then the decoder is flushed.
 * Built with AddressSanitizer, UndefinedBehaviorSanitizer and libFuzzer by
 * scripts/dev/vvc-fuzz.py; not part of the product.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <vvdec/vvdec.h>

static vvdecDecoder *g_dec;
static vvdecAccessUnit *g_au;

static void open_decoder(void) {
    vvdecParams p;
    vvdec_params_default(&p);
    p.threads = 1;
    p.logLevel = VVDEC_SILENT;
    p.errHandlingFlags = VVDEC_ERR_HANDLING_TRY_CONTINUE;
    g_dec = vvdec_decoder_open(&p);
}

/* Read every sample of every plane: a picture the decoder hands out must be fully readable. */
static uint32_t touch(const vvdecFrame *f) {
    uint32_t sum = 0;
    for (int c = 0; c < 3; c++) {
        const vvdecPlane *pl = &f->planes[c];
        if (!pl->ptr) continue;
        const size_t row = (size_t)pl->width * pl->bytesPerSample;
        for (uint32_t y = 0; y < pl->height; y++) {
            const uint8_t *p = (const uint8_t *)pl->ptr + (size_t)y * pl->stride;
            for (size_t x = 0; x < row; x += 61) sum += p[x];
        }
    }
    return sum;
}

static uint32_t drain(vvdecAccessUnit *au, bool flush) {
    uint32_t sum = 0;
    for (int round = 0; round < 64; round++) {
        vvdecFrame *f = NULL;
        const int res = flush ? vvdec_flush(g_dec, &f) : vvdec_decode(g_dec, au, &f);
        if (au) au->payloadUsedSize = 0;
        if (f) {
            sum += touch(f);
            vvdec_frame_unref(g_dec, f);
        }
        if (res != VVDEC_OK || !f) break;
    }
    return sum;
}

static void feed(const uint8_t *buf, size_t n, int64_t pts) {
    if (!n) return;
    if (n > (size_t)g_au->payloadSize) {
        vvdec_accessUnit_free_payload(g_au);
        vvdec_accessUnit_alloc_payload(g_au, (int)n);
        if (!g_au->payload) return;
    }
    memcpy(g_au->payload, buf, n);
    g_au->payloadUsedSize = (int)n;
    g_au->cts = (uint64_t)pts;
    g_au->ctsValid = true;
    g_au->dtsValid = false;
    (void)drain(g_au, false);
}

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size > (16u << 20)) return 0;
    if (!g_au) {
        g_au = vvdec_accessUnit_alloc();
        vvdec_accessUnit_alloc_payload(g_au, 1 << 20);
    }
    if (!g_dec) open_decoder();
    if (!g_dec || !g_au || !g_au->payload) return 0;

    size_t start = 0;
    int64_t pts = 0;
    for (size_t i = 0; i + 4 < size; i++) {
        /* start code 00 00 01 followed by a NAL header whose type (bits 7..3 of byte 2) is 20 */
        if (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 && (data[i + 4] >> 3) == 20 && i > start) {
            size_t end = (i > 0 && data[i - 1] == 0) ? i - 1 : i;
            if (end > start) feed(data + start, end - start, pts++);
            start = end;
        }
    }
    feed(data + start, size - start, pts);
    (void)drain(NULL, true);
    /* a fresh decoder for the next input: the helper reopens after every flush too */
    vvdec_decoder_close(g_dec);
    g_dec = NULL;
    return 0;
}
