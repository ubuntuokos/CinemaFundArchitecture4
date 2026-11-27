//! Highlight information (HLI) of a menu navigation pack (CFA4 #1812, D1): the button table that
//! lives in the PCI packet of every menu VOBU.
//!
//! Field order and widths follow libdvdread's `nav_read.c` (`navRead_PCI`, GPL-2.0-or-later,
//! (C) 2000-2001 Björn Englund, Håkan Hjort) and the comments of `nav_types.h`: the HLI starts
//! 0x60 bytes into the PCI packet (after `pci_gi` 60 bytes and `nsml_agli` 36 bytes) and is
//! 694 bytes long: `hl_gi` 22, button colour table 24, 36 button entries of 18 bytes.

use crate::vm::Cmd;
use crate::{Error, Result};

/// The PCI packet starts at this offset of a navigation sector (pack header, system header,
/// PES header and the sub-stream id of private stream 2 come first).
pub(crate) const PCI_AT: usize = 0x2D;
/// The HLI starts this far into the PCI packet.
const HLI_IN_PCI: usize = 0x60;
/// Most buttons of one group.
pub(crate) const MAX_BUTTONS: usize = 36;

/// One button of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Button {
    /// Left, top, right, bottom of the highlighted area in the 720 x 480/576 picture (inclusive).
    pub rect: (u16, u16, u16, u16),
    /// Colour set 1..=3 (a row of the colour table).
    pub color: u8,
    /// Activate the button as soon as it is selected.
    pub auto_action: bool,
    /// Neighbours as button numbers (1-based); 0 means "stay".
    pub up: u8,
    pub down: u8,
    pub left: u8,
    pub right: u8,
    /// What activating the button does (a VM command, see [`crate::vm`]).
    pub cmd: Cmd,
}

/// The colours of one set: four 4-bit palette indexes (background, pattern, emphasis 1, emphasis 2
/// as the subpicture's four colours 0..3) and four 4-bit contrasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColorSet {
    pub index: [u8; 4],
    pub alpha: [u8; 4],
}

impl ColorSet {
    /// The 32-bit table word `[Ci3 Ci2 Ci1 Ci0 A3 A2 A1 A0]`, a nibble each.
    fn word(self) -> u32 {
        let n = |v: u8| u32::from(v & 0xF);
        n(self.index[3]) << 28
            | n(self.index[2]) << 24
            | n(self.index[1]) << 20
            | n(self.index[0]) << 16
            | n(self.alpha[3]) << 12
            | n(self.alpha[2]) << 8
            | n(self.alpha[1]) << 4
            | n(self.alpha[0])
    }
}

/// The HLI of a menu: one button group, the same for every VOBU of the menu.
#[derive(Clone, Debug)]
pub(crate) struct Hli {
    /// `hli_ss`: 1 new highlight information, 2 the same as in the previous VOBU.
    pub status: u16,
    /// Start and end of the highlight in 90 kHz PTM units (the VOBU's own start and end).
    pub start_ptm: u32,
    pub end_ptm: u32,
    /// The group holds 16:9 buttons (otherwise 4:3).
    pub wide: bool,
    /// For colour sets 1..=3: `[selected, activated]`.
    pub colors: [[ColorSet; 2]; 3],
    pub buttons: Vec<Button>,
    /// Button selected on entry (1-based), 0: none forced.
    pub forced_select: u8,
}

fn put32(b: &mut [u8], at: usize, v: u32) -> Result<()> {
    b.get_mut(at..at + 4).ok_or_else(|| Error::Vob("HLI outside the navigation sector".into()))?.copy_from_slice(&v.to_be_bytes());
    Ok(())
}

/// Write the HLI into the PCI of the navigation sector `sector` (2048 bytes).
pub(crate) fn write(sector: &mut [u8], hli: &Hli) -> Result<()> {
    if hli.buttons.is_empty() || hli.buttons.len() > MAX_BUTTONS {
        return Err(Error::Vob(format!("a menu needs 1..={MAX_BUTTONS} buttons, got {}", hli.buttons.len())));
    }
    let n = hli.buttons.len();
    for (i, b) in hli.buttons.iter().enumerate() {
        let (x0, y0, x1, y1) = b.rect;
        if x0 >= x1 || y0 >= y1 || x1 > 1023 || y1 > 1023 {
            return Err(Error::Vob(format!("button {}: bad rectangle", i + 1)));
        }
        if !(1..=3).contains(&b.color) {
            return Err(Error::Vob(format!("button {}: colour set must be 1..=3", i + 1)));
        }
        for t in [b.up, b.down, b.left, b.right] {
            if usize::from(t) > n {
                return Err(Error::Vob(format!("button {}: neighbour {t} does not exist", i + 1)));
            }
        }
    }
    if usize::from(hli.forced_select) > n {
        return Err(Error::Vob("forced button does not exist".into()));
    }
    let base = PCI_AT + HLI_IN_PCI;
    let end = base + 22 + 24 + MAX_BUTTONS * 18;
    let region = sector.get_mut(base..end).ok_or_else(|| Error::Vob("navigation sector too short".into()))?;
    region.fill(0);
    // hl_gi
    region[0..2].copy_from_slice(&hli.status.to_be_bytes());
    put32(region, 2, hli.start_ptm)?;
    put32(region, 6, hli.end_ptm)?;
    put32(region, 10, hli.end_ptm)?; // buttons stay selectable to the end
    region[14] = 1 << 4 | u8::from(hli.wide); // one group; its display type (001b = 16:9)
    region[16] = 0; // btn_ofn
    region[17] = n as u8;
    region[18] = n as u8;
    region[20] = hli.forced_select;
    // colour table
    for (i, set) in hli.colors.iter().enumerate() {
        for (j, c) in set.iter().enumerate() {
            put32(region, 22 + 8 * i + 4 * j, c.word())?;
        }
    }
    // button table
    for (i, b) in hli.buttons.iter().enumerate() {
        let at = 46 + 18 * i;
        let (x0, y0, x1, y1) = b.rect;
        let a = u32::from(b.color) << 22 | u32::from(x0) << 12 | u32::from(x1);
        let c = u32::from(b.auto_action) << 22 | u32::from(y0) << 12 | u32::from(y1);
        region[at..at + 3].copy_from_slice(&a.to_be_bytes()[1..]);
        region[at + 3..at + 6].copy_from_slice(&c.to_be_bytes()[1..]);
        region[at + 6..at + 10].copy_from_slice(&[b.up, b.down, b.left, b.right]);
        region[at + 10..at + 18].copy_from_slice(&b.cmd);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SECTOR, vm};

    /// A bit reader in the field order of libdvdread's `navRead_PCI`, to read back what `write` made.
    struct Bits<'a> {
        data: &'a [u8],
        pos: usize,
    }
    impl Bits<'_> {
        fn get(&mut self, n: usize) -> u32 {
            let mut v = 0u32;
            for _ in 0..n {
                let bit = (self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1;
                v = v << 1 | u32::from(bit);
                self.pos += 1;
            }
            v
        }
    }

    fn sample() -> Hli {
        let set = |i: u8| ColorSet { index: [i, i + 1, i + 2, i + 3], alpha: [0, 15, 15, 15] };
        let btn = |x0, y0, x1, y1, up, down, cmd| Button { rect: (x0, y0, x1, y1), color: 1, auto_action: false, up, down, left: 0, right: 0, cmd };
        Hli {
            status: 1,
            start_ptm: 0x1111_2222,
            end_ptm: 0x3333_4444,
            wide: true,
            colors: [[set(1), set(5)], [set(2), set(6)], [set(3), set(7)]],
            buttons: vec![
                btn(80, 100, 400, 140, 0, 2, vm::jump_tt(1)),
                btn(80, 180, 400, 220, 1, 0, vm::call_ss(vm::Menu::Vtsm { vts: 1, title: 1, menu: 3 }, 1)),
            ],
            forced_select: 1,
        }
    }

    #[test]
    fn the_hli_reads_back_in_libdvdread_field_order() {
        let mut sector = vec![0xEEu8; SECTOR];
        let hli = sample();
        write(&mut sector, &hli).expect("write");
        let mut r = Bits { data: &sector, pos: (PCI_AT + HLI_IN_PCI) * 8 };
        assert_eq!(r.get(16), 1, "hli_ss");
        assert_eq!(r.get(32), 0x1111_2222);
        assert_eq!(r.get(32), 0x3333_4444);
        assert_eq!(r.get(32), 0x3333_4444, "btn_se_e_ptm");
        assert_eq!((r.get(2), r.get(2), r.get(1), r.get(3)), (0, 1, 0, 1), "one group, 16:9");
        assert_eq!((r.get(1), r.get(3), r.get(1), r.get(3)), (0, 0, 0, 0));
        assert_eq!((r.get(8), r.get(8), r.get(8), r.get(8), r.get(8), r.get(8)), (0, 2, 2, 0, 1, 0), "ofn ns nsl zero fosl foac");
        // colour table: [set][select, action]
        let word = |s: ColorSet| s.word();
        for i in 0..3 {
            for j in 0..2 {
                assert_eq!(r.get(32), word(hli.colors[i][j]));
            }
        }
        // the first button, bit by bit
        assert_eq!((r.get(2), r.get(10), r.get(2), r.get(10)), (1, 80, 0, 400), "coln x_start zero x_end");
        assert_eq!((r.get(2), r.get(10), r.get(2), r.get(10)), (0, 100, 0, 140), "auto y_start zero y_end");
        assert_eq!((r.get(2), r.get(6), r.get(2), r.get(6), r.get(2), r.get(6), r.get(2), r.get(6)), (0, 0, 0, 2, 0, 0, 0, 0));
        let cmd: Vec<u32> = (0..8).map(|_| r.get(8)).collect();
        assert_eq!(cmd, vm::jump_tt(1).iter().map(|&b| u32::from(b)).collect::<Vec<_>>());
        // the second button's command and neighbours
        let _ = (r.get(2), r.get(10), r.get(2), r.get(10), r.get(2), r.get(10), r.get(2), r.get(10));
        assert_eq!((r.get(2), r.get(6), r.get(2), r.get(6)), (0, 1, 0, 0));
        // the rest of the sector is untouched, the table is zero past the used buttons
        let end = PCI_AT + HLI_IN_PCI + 694;
        assert!(sector[..PCI_AT].iter().all(|&b| b == 0xEE));
        assert!(sector[end..].iter().all(|&b| b == 0xEE));
        let third = PCI_AT + HLI_IN_PCI + 22 + 24 + 2 * 18;
        assert!(sector[third..end].iter().all(|&b| b == 0));
    }

    #[test]
    fn bad_menus_are_refused() {
        let mut sector = vec![0u8; SECTOR];
        let mut h = sample();
        h.buttons[0].up = 3;
        assert!(write(&mut sector, &h).is_err(), "neighbour out of range");
        let mut h = sample();
        h.buttons[1].rect = (10, 10, 10, 20);
        assert!(write(&mut sector, &h).is_err(), "empty rectangle");
        let mut h = sample();
        h.buttons.clear();
        assert!(write(&mut sector, &h).is_err(), "no buttons");
        let mut h = sample();
        h.buttons[0].color = 0;
        assert!(write(&mut sector, &h).is_err(), "colour set");
    }
}
