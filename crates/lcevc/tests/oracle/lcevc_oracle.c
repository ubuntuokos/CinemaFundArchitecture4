/* Test oracle driver for the LCEVC decoder in crates/lcevc (CFA4, GPL-3.0-or-later).
 *
 * Feeds V-Nova's LCEVCdec (BSD-3-Clause-Clear, built by scripts/ci/install-lcevc-oracle.sh) with
 * base pictures and LCEVC NAL units and writes the enhanced pictures. Never linked into or
 * shipped with CFA4.
 *
 * stdin:  u32 LE frame count, base format, base width, base height, output format, output width,
 *         output height; then per frame: u32 LE NAL size, the NAL unit (Annex B, start code
 *         included), the base picture planes (tightly packed, 16-bit LE above 8 bits).
 * stdout: per frame, the output picture planes (tightly packed).
 * LCEVC formats: 1001 I420_8, 1002 I420_10_LE, 1201 I422_8, 1202 I422_10_LE, 1401 I444_8,
 * 1402 I444_10_LE, 5001 GRAY_8, 5002 GRAY_10_LE. */
#include <LCEVC/lcevc_dec.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int readAll(void* dst, size_t size)
{
    return fread(dst, 1, size, stdin) == size;
}

static uint32_t readU32(void)
{
    uint8_t b[4];
    if (!readAll(b, 4)) {
        fprintf(stderr, "lcevc_oracle: truncated input\n");
        exit(2);
    }
    return (uint32_t)b[0] | ((uint32_t)b[1] << 8) | ((uint32_t)b[2] << 16) | ((uint32_t)b[3] << 24);
}

typedef struct Layout
{
    uint32_t planes;
    uint32_t width[3];
    uint32_t height[3];
    uint32_t bytes;
    size_t size;
} Layout;

static Layout layout(uint32_t format, uint32_t width, uint32_t height)
{
    Layout l = {0};
    const uint32_t family = format / 100;
    l.bytes = (format % 100) == 1 ? 1 : 2;
    l.planes = family == 50 ? 1 : 3;
    l.width[0] = width;
    l.height[0] = height;
    for (uint32_t p = 1; p < l.planes; ++p) {
        l.width[p] = family == 14 ? width : width / 2;
        l.height[p] = family == 10 ? height / 2 : height;
    }
    for (uint32_t p = 0; p < l.planes; ++p) {
        l.size += (size_t)l.width[p] * l.height[p] * l.bytes;
    }
    return l;
}

static LCEVC_PictureHandle allocPicture(LCEVC_DecoderHandle dec, uint32_t format, const Layout* l,
                                        uint8_t* buffer, uint32_t width, uint32_t height)
{
    LCEVC_PictureDesc desc;
    if (LCEVC_DefaultPictureDesc(&desc, (LCEVC_ColorFormat)format, width, height) != LCEVC_Success) {
        fprintf(stderr, "lcevc_oracle: bad picture format %u\n", format);
        exit(2);
    }
    LCEVC_PicturePlaneDesc planes[3];
    uint8_t* p = buffer;
    for (uint32_t i = 0; i < l->planes; ++i) {
        planes[i].firstSample = p;
        planes[i].rowByteStride = l->width[i] * l->bytes;
        p += (size_t)l->width[i] * l->height[i] * l->bytes;
    }
    LCEVC_PictureHandle pic = {0};
    if (LCEVC_AllocPictureExternal(dec, &desc, NULL, planes, &pic) != LCEVC_Success) {
        fprintf(stderr, "lcevc_oracle: AllocPictureExternal failed\n");
        exit(2);
    }
    return pic;
}

int main(void)
{
    const uint32_t frames = readU32();
    const uint32_t baseFormat = readU32();
    const uint32_t baseWidth = readU32();
    const uint32_t baseHeight = readU32();
    const uint32_t outFormat = readU32();
    const uint32_t outWidth = readU32();
    const uint32_t outHeight = readU32();
    const Layout baseLayout = layout(baseFormat, baseWidth, baseHeight);
    const Layout outLayout = layout(outFormat, outWidth, outHeight);

    LCEVC_DecoderHandle dec = {0};
    if (LCEVC_CreateDecoder(&dec, (LCEVC_AccelContextHandle){0}) != LCEVC_Success) {
        fprintf(stderr, "lcevc_oracle: CreateDecoder failed\n");
        return 2;
    }
    LCEVC_ConfigureDecoderBool(dec, "disable_simd", true);
    LCEVC_ConfigureDecoderInt(dec, "threads", 1);
    LCEVC_ConfigureDecoderBool(dec, "allow_dithering", false);
    LCEVC_ConfigureDecoderInt(dec, "log_level", 1);
    if (LCEVC_InitializeDecoder(dec) != LCEVC_Success) {
        fprintf(stderr, "lcevc_oracle: InitializeDecoder failed\n");
        return 2;
    }

    uint8_t* baseBuffer = malloc(baseLayout.size);
    uint8_t* outBuffer = malloc(outLayout.size);
    if (!baseBuffer || !outBuffer) {
        return 2;
    }
    for (uint32_t frame = 0; frame < frames; ++frame) {
        const uint32_t nalSize = readU32();
        uint8_t* nal = malloc(nalSize ? nalSize : 1);
        if (!nal || !readAll(nal, nalSize) || !readAll(baseBuffer, baseLayout.size)) {
            fprintf(stderr, "lcevc_oracle: truncated frame %u\n", frame);
            return 2;
        }
        const uint64_t timestamp = (uint64_t)frame + 1;
        if (nalSize > 0 &&
            LCEVC_SendDecoderEnhancementData(dec, timestamp, nal, nalSize) != LCEVC_Success) {
            fprintf(stderr, "lcevc_oracle: SendDecoderEnhancementData failed (frame %u)\n", frame);
            return 3;
        }
        free(nal);
        LCEVC_PictureHandle base =
            allocPicture(dec, baseFormat, &baseLayout, baseBuffer, baseWidth, baseHeight);
        LCEVC_PictureHandle out =
            allocPicture(dec, outFormat, &outLayout, outBuffer, outWidth, outHeight);
        if (LCEVC_SendDecoderBase(dec, timestamp, base, 1000000, NULL) != LCEVC_Success) {
            fprintf(stderr, "lcevc_oracle: SendDecoderBase failed (frame %u)\n", frame);
            return 3;
        }
        if (LCEVC_SendDecoderPicture(dec, out) != LCEVC_Success) {
            fprintf(stderr, "lcevc_oracle: SendDecoderPicture failed (frame %u)\n", frame);
            return 3;
        }
        LCEVC_PictureHandle got = {0};
        LCEVC_DecodeInformation info;
        LCEVC_ReturnCode rc = LCEVC_Again;
        for (int tries = 0; tries < 100000 && rc == LCEVC_Again; ++tries) {
            rc = LCEVC_ReceiveDecoderPicture(dec, &got, &info);
        }
        if (rc != LCEVC_Success) {
            fprintf(stderr, "lcevc_oracle: ReceiveDecoderPicture returned %d (frame %u)\n", rc, frame);
            return 3;
        }
        if (!info.enhanced) {
            fprintf(stderr, "lcevc_oracle: frame %u was not enhanced\n", frame);
            return 4;
        }
        LCEVC_PictureHandle doneBase = {0};
        while (LCEVC_ReceiveDecoderBase(dec, &doneBase) == LCEVC_Success) {
            LCEVC_FreePicture(dec, doneBase);
        }
        if (fwrite(outBuffer, 1, outLayout.size, stdout) != outLayout.size) {
            return 2;
        }
        LCEVC_FreePicture(dec, got);
    }
    fflush(stdout);
    LCEVC_DestroyDecoder(dec);
    free(baseBuffer);
    free(outBuffer);
    return 0;
}
