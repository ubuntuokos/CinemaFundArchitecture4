use crate::{
    avcodec::{AVCodecContext, AVCodecID, AVPacket},
    error::*,
    ffi,
    shared::*,
};

wrap!(AVCodecParserContext: ffi::AVCodecParserContext);

impl AVCodecParserContext {
    /// Allocate a [`AVCodecParserContext`] with given [`AVCodecID`].
    pub fn init(codec_id: AVCodecID) -> Option<Self> {
        // ffmpeg9 changed the parameter type from `u32` to `enum AVCodecID`.
        unsafe { ffi::av_parser_init(codec_id as _) }
            .upgrade()
            .map(|x| unsafe { Self::from_raw(x) })
    }

    /// Parse a packet.
    ///
    /// Return `Err(_)` On failure, `bool` field of returned tuple means if
    /// packet is ready, `usize` field of returned tuple means the offset of the
    /// data being parsed.
    ///
    /// The timing fields of `packet`(pts, dts and pos) feed the parser and get
    /// preserved, everything else it held before is replaced by the result of
    /// this call.
    ///
    /// Note: if `data.len()` exceeds [`i32::MAX`], this function returns
    /// [`RsmpegError::TryFromIntError`].
    pub fn parse_packet(
        &mut self,
        codec_context: &mut AVCodecContext,
        packet: &mut AVPacket,
        data: &[u8],
    ) -> Result<(bool, usize)> {
        // According to the documentation of `av_parser_parse2()`:
        //
        // `buf_size`: Input length in bytes **without** the padding. I.e. the
        // full buffer size is assumed to be `buf_size` +
        // `AV_INPUT_BUFFER_PADDING_SIZE`.
        //
        // So the given `data` is copied into a padded buffer here, otherwise the
        // bitstream readers inside FFmpeg read out of bounds.
        let padding = ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
        let mut buffer = Vec::with_capacity(data.len() + padding);
        buffer.extend_from_slice(data);
        buffer.resize(data.len() + padding, 0);

        // The timing fields of `packet` are the input of `av_parser_parse2()`,
        // they are restored after the packet gets wiped below.
        let (pts, dts, pos) = (packet.pts, packet.dts, packet.pos);

        // `av_parser_parse2()` doesn't read the initial value of these two.
        let mut packet_data = std::ptr::null_mut();
        let mut packet_size = 0;
        let offset = unsafe {
            ffi::av_parser_parse2(
                self.as_mut_ptr(),
                codec_context.as_mut_ptr(),
                &mut packet_data,
                &mut packet_size,
                buffer.as_ptr(),
                data.len().try_into()?,
                pts,
                dts,
                pos,
            )
        }
        .upgrade()?;

        // Empty the packet. The buffer returned by `av_parser_parse2()` is
        // owned by either `buffer` above(only borrowed here) or the parser
        // itself(which frees or reuses it later), so the parsed data is copied
        // into a buffer owned by the packet below.
        unsafe { ffi::av_packet_unref(packet.as_mut_ptr()) };
        packet.set_pts(pts);
        packet.set_dts(dts);
        packet.set_pos(pos);

        // No complete packet has been parsed yet.
        if packet_data.is_null() || packet_size == 0 {
            return Ok((false, offset as usize));
        }

        // Allocate the buffer owned by `packet`, then fill it with the parsed
        // data. Note that the `av_packet_unref()` above makes sure that nothing
        // the packet held before is leaked here.
        unsafe { ffi::av_new_packet(packet.as_mut_ptr(), packet_size) }
            .upgrade()
            .map_err(RsmpegError::AVError)?;
        unsafe {
            std::ptr::copy_nonoverlapping(packet_data, packet.data, packet_size as usize);
        }

        Ok((true, offset as usize))
    }
}

impl Drop for AVCodecParserContext {
    fn drop(&mut self) {
        unsafe { ffi::av_parser_close(self.as_mut_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avcodec::AVCodec;
    use std::fs;

    /// The data [`AVCodecParserContext::parse_packet()`] puts into the packet
    /// must outlive both the input buffer it was parsed from and the parser
    /// itself, otherwise every caller keeping the packet around reads freed
    /// memory.
    #[test]
    fn test_parse_packet_owns_its_data() {
        let Some(decoder) = AVCodec::find_decoder(ffi::AV_CODEC_ID_MPEG2VIDEO) else {
            println!("skip: mpeg2video decoder is not available");
            return;
        };
        let mut codec_context = AVCodecContext::new(&decoder);
        let Some(mut parser) = AVCodecParserContext::init(ffi::AV_CODEC_ID_MPEG2VIDEO) else {
            println!("skip: mpeg2video parser is not available");
            return;
        };

        let mut packet = AVPacket::new();
        // Snapshot of the beginning of the parsed data, taken while everything
        // involved in the parsing is still alive.
        let snapshot = {
            // The input buffer is dropped at the end of this scope.
            let data = fs::read("tests/assets/vids/centaur.mpg").unwrap();

            let mut parsed = false;
            let mut offset = 0;
            while !parsed {
                let (ready, used) = parser
                    .parse_packet(
                        &mut codec_context,
                        &mut packet,
                        &data[offset..offset + 2048],
                    )
                    .unwrap();
                parsed = ready;
                offset += used.max(1);
                assert!(parsed || offset + 2048 <= data.len());
            }

            assert!(packet.size > 0);
            let data = unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
            data[..128.min(data.len())].to_vec()
        };

        // The parser owns the buffer it hands back for some stream kinds, so
        // dropping it here frees the memory a packet aliasing it points to.
        drop(parser);

        // Take the memory freed above back, a packet aliasing it would read all
        // the `0xAB` written here.
        let mut garbage = vec![0xABu8; 4 * 1024 * 1024];
        std::hint::black_box(&mut garbage);

        let data = unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
        assert_eq!(&data[..snapshot.len()], &snapshot[..]);
        assert!(data.windows(3).any(|x| x == [0, 0, 1]));
    }

    /// Parsing the whole file must be able to produce complete packets.
    #[test]
    fn test_parse_packet_stream() {
        let Some(decoder) = AVCodec::find_decoder(ffi::AV_CODEC_ID_MPEG2VIDEO) else {
            println!("skip: mpeg2video decoder is not available");
            return;
        };
        let mut codec_context = AVCodecContext::new(&decoder);
        let Some(mut parser) = AVCodecParserContext::init(ffi::AV_CODEC_ID_MPEG2VIDEO) else {
            println!("skip: mpeg2video parser is not available");
            return;
        };

        let data = fs::read("tests/assets/vids/centaur.mpg").unwrap();
        let mut packet = AVPacket::new();
        let mut packets = 0;
        let mut offset = 0;
        while offset < data.len() {
            let (ready, used) = parser
                .parse_packet(&mut codec_context, &mut packet, &data[offset..])
                .unwrap();
            packets += i32::from(ready);
            offset += used.max(1);
        }
        assert!(packets > 1);
    }
}
