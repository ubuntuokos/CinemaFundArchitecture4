//! Ogg page writer (RFC 3533 §6) for one logical bitstream.

use std::io::{self, Write};

use crate::crc32;

/// Largest page body the writer aims for before starting a new page.
const TARGET_PAGE_BYTES: usize = 4096;

/// Packs packets into Ogg pages: lacing, continuation, granule positions, CRC.
///
/// Packets are buffered until a page fills (255 lacing values or about 4 KiB), [`Self::flush_page`]
/// is called, or [`Self::finish`] writes the last page with the end-of-stream flag.
pub struct OggWriter<W: Write> {
    out: W,
    serial: u32,
    seq: u32,
    first: bool,
    lacing: Vec<u8>,
    body: Vec<u8>,
    /// Granule of the last packet completed on the pending page.
    granule: Option<u64>,
    /// The pending page starts with the continuation of a packet.
    continued: bool,
}

impl<W: Write> OggWriter<W> {
    pub fn new(out: W, serial: u32) -> Self {
        OggWriter { out, serial, seq: 0, first: true, lacing: Vec::new(), body: Vec::new(), granule: None, continued: false }
    }

    /// Adds one packet; `granule` is the granule position at its end.
    pub fn write_packet(&mut self, packet: &[u8], granule: u64) -> io::Result<()> {
        if self.body.len() >= TARGET_PAGE_BYTES {
            self.flush_page()?;
        }
        let full = packet.len() / 255;
        let last = (packet.len() % 255) as u8;
        let mut off = 0;
        for k in 0..=full {
            if self.lacing.len() == 255 {
                self.write_page(false)?;
                // the next page continues a packet only when this one is split across the break
                self.continued = k > 0;
            }
            let v = if k < full { 255 } else { last };
            self.lacing.push(v);
            self.body.extend_from_slice(packet.get(off..off + v as usize).unwrap_or(&[]));
            off += v as usize;
        }
        self.granule = Some(granule);
        Ok(())
    }

    /// Ends the pending page (if any), e.g. after the codec headers.
    pub fn flush_page(&mut self) -> io::Result<()> {
        if self.lacing.is_empty() {
            return Ok(());
        }
        self.write_page(false)
    }

    /// Writes the pending packets as the final (end-of-stream) page and returns the sink.
    pub fn finish(mut self) -> io::Result<W> {
        self.write_page(true)?;
        self.out.flush()?;
        Ok(self.out)
    }

    fn write_page(&mut self, eos: bool) -> io::Result<()> {
        let mut flags = 0u8;
        if self.continued {
            flags |= 1;
        }
        if self.first {
            flags |= 2;
        }
        if eos {
            flags |= 4;
        }
        // A page on which no packet completes carries granule -1.
        let granule = self.granule.take().unwrap_or(u64::MAX);
        let mut page = Vec::with_capacity(27 + self.lacing.len() + self.body.len());
        page.extend_from_slice(b"OggS");
        page.push(0);
        page.push(flags);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&self.serial.to_le_bytes());
        page.extend_from_slice(&self.seq.to_le_bytes());
        page.extend_from_slice(&[0; 4]);
        page.push(self.lacing.len() as u8);
        page.extend_from_slice(&self.lacing);
        page.extend_from_slice(&self.body);
        let crc = crc32(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        self.out.write_all(&page)?;
        self.seq = self.seq.wrapping_add(1);
        self.first = false;
        self.continued = false;
        self.lacing.clear();
        self.body.clear();
        Ok(())
    }
}
