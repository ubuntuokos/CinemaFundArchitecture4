/*
 * The Musepack test oracle of CFA4 (crates/musepack): decodes an SV7 or SV8 `.mpc` file with
 * libmpcdec (musepack_src_r475, BSD-3-Clause, floating-point build) and writes the samples as
 * raw interleaved 32-bit little-endian floats exactly as libmpcdec returns them, without the
 * 16-bit conversion of mpcdec. It is an external test tool, never linked into or shipped with
 * CFA4 (scripts/ci/install-musepack-oracle.sh builds it, together with the encoder mpcenc).
 *
 *   cfa4-mpc-oracle in.mpc out.f32
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <stdio.h>

#include <mpc/mpcdec.h>

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: cfa4-mpc-oracle in.mpc out.f32\n");
        return 2;
    }
    mpc_reader reader;
    if (mpc_reader_init_stdio(&reader, argv[1]) != MPC_STATUS_OK)
        return 1;
    mpc_demux *demux = mpc_demux_init(&reader);
    if (!demux)
        return 1;
    FILE *out = fopen(argv[2], "wb");
    if (!out)
        return 1;
    mpc_streaminfo si;
    mpc_demux_get_info(demux, &si);
    MPC_SAMPLE_FORMAT buf[MPC_DECODER_BUFFER_LENGTH];
    int status = 0;
    for (;;) {
        mpc_frame_info frame;
        frame.buffer = buf;
        if (mpc_demux_decode(demux, &frame) != MPC_STATUS_OK) {
            status = 1;
            break;
        }
        if (frame.bits == -1)
            break;
        size_t n = (size_t)frame.samples * si.channels;
        if (fwrite(buf, sizeof buf[0], n, out) != n) {
            status = 1;
            break;
        }
    }
    mpc_demux_exit(demux);
    mpc_reader_exit_stdio(&reader);
    return fclose(out) == 0 ? status : 1;
}
