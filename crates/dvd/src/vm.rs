//! DVD virtual machine commands (8 bytes, big-endian) for the menu authoring (CFA4 #1812, D1).
//!
//! The bit positions are the ones libdvdnav's `vmcmd.c` decodes (libdvdnav, GPL-2.0-or-later,
//! (C) 2000, 2001 Martin Norbäck, Håkan Hjort, 2002-2004 the dvdnav project; derived from Ogle):
//! the 64-bit instruction is numbered from bit 63 (the top of byte 0) down to bit 0, a field is
//! given as (most significant bit, width). Only the narrow set the menus need is encoded; every
//! other command is out of scope (no compare/`if`, no GPRM arithmetic besides `mov`).
//!
//! Group (bits 63..61): 0 special, 1 link (bit 60 = 0) / jump and call (bit 60 = 1),
//! 2 set system register, 3 set general register.

/// One 8-byte VM command.
pub type Cmd = [u8; 8];

/// `no link` / link sub-instruction codes (bits 7..0 of a `LinkSub`).
const LINKSUB_RSM: u8 = 16;

/// Place `value` into the field whose top bit is `msb` and that is `width` bits wide.
fn set(word: &mut u64, msb: u32, width: u32, value: u64) {
    let shift = msb + 1 - width;
    let mask = ((1u64 << width) - 1) << shift;
    *word = (*word & !mask) | ((value << shift) & mask);
}

fn bytes(word: u64) -> Cmd {
    word.to_be_bytes()
}

/// `Nop`.
pub const NOP: Cmd = [0; 8];

/// `Break`: ends the command table.
pub fn brk() -> Cmd {
    let mut w = 0;
    set(&mut w, 51, 4, 2);
    bytes(w)
}

/// `Goto line` (1-based line of the same table).
pub fn goto(line: u8) -> Cmd {
    let mut w = 0;
    set(&mut w, 51, 4, 1);
    set(&mut w, 7, 8, u64::from(line));
    bytes(w)
}

fn link_header(op: u64) -> u64 {
    let mut w = 0;
    set(&mut w, 63, 3, 1);
    set(&mut w, 51, 4, op);
    w
}

/// `LinkPGCN pgcn`.
pub fn link_pgcn(pgcn: u16) -> Cmd {
    let mut w = link_header(4);
    set(&mut w, 14, 15, u64::from(pgcn));
    bytes(w)
}

/// `LinkPTT ptt` (button 0 = keep the highlight).
pub fn link_ptt(ptt: u16, button: u8) -> Cmd {
    let mut w = link_header(5);
    set(&mut w, 9, 10, u64::from(ptt));
    set(&mut w, 15, 6, u64::from(button));
    bytes(w)
}

/// `LinkPGN pgn`.
pub fn link_pgn(pgn: u8, button: u8) -> Cmd {
    let mut w = link_header(6);
    set(&mut w, 6, 7, u64::from(pgn));
    set(&mut w, 15, 6, u64::from(button));
    bytes(w)
}

/// `LinkCN cn`.
pub fn link_cn(cn: u8, button: u8) -> Cmd {
    let mut w = link_header(7);
    set(&mut w, 7, 8, u64::from(cn));
    set(&mut w, 15, 6, u64::from(button));
    bytes(w)
}

/// A `LinkSub` (`LinkTopC` 1, `LinkNextC` 2, `LinkPrevC` 3, `LinkTopPG` 5, …, `RSM` 16).
fn link_sub(code: u8, button: u8) -> Cmd {
    let mut w = link_header(1);
    set(&mut w, 7, 8, u64::from(code));
    set(&mut w, 15, 6, u64::from(button));
    bytes(w)
}

/// `LinkTopC`: replay the current cell (a menu loops with this).
pub fn link_top_cell() -> Cmd {
    link_sub(1, 0)
}

/// `LinkTopPGC`: restart the current program chain.
pub fn link_top_pgc() -> Cmd {
    link_sub(9, 0)
}

/// `RSM`: resume playback after a menu call.
pub fn resume() -> Cmd {
    link_sub(LINKSUB_RSM, 0)
}

fn jump_header(op: u64) -> u64 {
    let mut w = 0;
    set(&mut w, 63, 3, 1);
    set(&mut w, 60, 1, 1);
    set(&mut w, 51, 4, op);
    w
}

/// `Exit`.
pub fn exit() -> Cmd {
    bytes(jump_header(1))
}

/// `JumpTT title` (video manager, title number 1..=99).
pub fn jump_tt(title: u8) -> Cmd {
    let mut w = jump_header(2);
    set(&mut w, 22, 7, u64::from(title));
    bytes(w)
}

/// `JumpVTS_TT title` (title within the current title set).
pub fn jump_vts_tt(title: u8) -> Cmd {
    let mut w = jump_header(3);
    set(&mut w, 22, 7, u64::from(title));
    bytes(w)
}

/// `JumpVTS_PTT title:ptt`.
pub fn jump_vts_ptt(title: u8, ptt: u16) -> Cmd {
    let mut w = jump_header(5);
    set(&mut w, 22, 7, u64::from(title));
    set(&mut w, 41, 10, u64::from(ptt));
    bytes(w)
}

/// The menu a `JumpSS` / `CallSS` goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// First-play program chain (`JumpSS` / `CallSS` only).
    FirstPlay,
    /// A video manager menu by id (2 title, 3 root of the title set …; 1 is the VTS root of `CallSS`).
    Vmgm(u8),
    /// A title set menu: title set, title within it, menu id (3 PTT).
    Vtsm { vts: u8, title: u8, menu: u8 },
    /// A video manager menu by program chain number.
    VmgmPgc(u16),
}

/// `JumpSS`.
pub fn jump_ss(menu: Menu) -> Cmd {
    let mut w = jump_header(6);
    match menu {
        Menu::FirstPlay => set(&mut w, 23, 2, 0),
        Menu::Vmgm(id) => {
            set(&mut w, 23, 2, 1);
            set(&mut w, 19, 4, u64::from(id));
        }
        Menu::Vtsm { vts, title, menu } => {
            set(&mut w, 23, 2, 2);
            set(&mut w, 30, 7, u64::from(vts));
            set(&mut w, 38, 7, u64::from(title));
            set(&mut w, 19, 4, u64::from(menu));
        }
        Menu::VmgmPgc(pgc) => {
            set(&mut w, 23, 2, 3);
            set(&mut w, 46, 15, u64::from(pgc));
        }
    }
    bytes(w)
}

/// `CallSS` (the resume cell is where `RSM` returns to).
pub fn call_ss(menu: Menu, rsm_cell: u8) -> Cmd {
    let mut w = jump_header(8);
    set(&mut w, 31, 8, u64::from(rsm_cell));
    match menu {
        Menu::FirstPlay => set(&mut w, 23, 2, 0),
        // a `CallSS` to a title set menu names the menu only (the title set is the current one)
        Menu::Vmgm(id) => {
            set(&mut w, 23, 2, 1);
            set(&mut w, 19, 4, u64::from(id));
        }
        Menu::Vtsm { menu, .. } => {
            set(&mut w, 23, 2, 2);
            set(&mut w, 19, 4, u64::from(menu));
        }
        Menu::VmgmPgc(pgc) => {
            set(&mut w, 23, 2, 3);
            set(&mut w, 46, 15, u64::from(pgc));
        }
    }
    bytes(w)
}

fn set_system_header(op: u64, immediate: bool) -> u64 {
    let mut w = 0;
    set(&mut w, 63, 3, 2);
    set(&mut w, 60, 1, u64::from(immediate));
    set(&mut w, 59, 4, op);
    w
}

/// `SetHL_BTNN button` (system register 8, the highlighted button, 1..=36).
pub fn set_hl_btnn(button: u8) -> Cmd {
    let mut w = set_system_header(6, true);
    // the register holds the button number in its top 6 bits
    set(&mut w, 31, 6, u64::from(button));
    bytes(w)
}

/// `SetSTN`: audio, sub-picture and angle numbers; `None` leaves a register alone. The
/// sub-picture value holds the stream number in bits 0..5 and the display flag in bit 6.
pub fn set_stn(audio: Option<u8>, subpicture: Option<u8>, angle: Option<u8>) -> Cmd {
    let mut w = set_system_header(1, true);
    for (i, v) in [audio, subpicture, angle].into_iter().enumerate() {
        if let Some(v) = v {
            let start = 47 - 8 * (i as u32 + 1);
            set(&mut w, start, 1, 1);
            set(&mut w, start - 1, 7, u64::from(v));
        }
    }
    bytes(w)
}

/// `mov gN, value` (general register 0..=15).
pub fn mov_gprm(register: u8, value: u16) -> Cmd {
    let mut w = 0;
    set(&mut w, 63, 3, 3);
    set(&mut w, 60, 1, 1);
    set(&mut w, 59, 4, 1);
    set(&mut w, 35, 4, u64::from(register));
    set(&mut w, 31, 16, u64::from(value));
    bytes(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden bytes: each is the encoding libdvdnav's `vmcmd.c` decodes to the named mnemonic.
    #[test]
    fn nop_goto_break() {
        assert_eq!(NOP, [0; 8]);
        assert_eq!(goto(3), [0x00, 0x01, 0, 0, 0, 0, 0, 3]);
        assert_eq!(brk(), [0x00, 0x02, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn links() {
        assert_eq!(link_pgcn(1), [0x20, 0x04, 0, 0, 0, 0, 0x00, 0x01]);
        assert_eq!(link_pgcn(0x7FFF), [0x20, 0x04, 0, 0, 0, 0, 0x7F, 0xFF]);
        assert_eq!(link_ptt(0x203, 5), [0x20, 0x05, 0, 0, 0, 0, 5 << 2 | 0x02, 0x03]);
        assert_eq!(link_pgn(2, 0), [0x20, 0x06, 0, 0, 0, 0, 0, 2]);
        assert_eq!(link_cn(9, 4), [0x20, 0x07, 0, 0, 0, 0, 4 << 2, 9]);
        assert_eq!(link_top_cell(), [0x20, 0x01, 0, 0, 0, 0, 0, 1]);
        assert_eq!(link_top_pgc(), [0x20, 0x01, 0, 0, 0, 0, 0, 9]);
        assert_eq!(resume(), [0x20, 0x01, 0, 0, 0, 0, 0, 16]);
    }

    #[test]
    fn jumps() {
        assert_eq!(exit(), [0x30, 0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(jump_tt(1), [0x30, 0x02, 0, 0, 0, 1, 0, 0]);
        assert_eq!(jump_vts_tt(2), [0x30, 0x03, 0, 0, 0, 2, 0, 0]);
        assert_eq!(jump_vts_ptt(1, 0x105), [0x30, 0x05, 0x01, 0x05, 0, 1, 0, 0]);
    }

    #[test]
    fn jump_and_call_to_menus() {
        assert_eq!(jump_ss(Menu::FirstPlay), [0x30, 0x06, 0, 0, 0, 0, 0, 0]);
        assert_eq!(jump_ss(Menu::Vmgm(2)), [0x30, 0x06, 0, 0, 0, 0x42, 0, 0]);
        assert_eq!(jump_ss(Menu::Vtsm { vts: 1, title: 1, menu: 3 }), [0x30, 0x06, 0, 0x01, 0x01, 0x83, 0, 0]);
        assert_eq!(jump_ss(Menu::VmgmPgc(1)), [0x30, 0x06, 0, 0x01, 0x00, 0xC0, 0, 0]);
        assert_eq!(call_ss(Menu::Vmgm(2), 1), [0x30, 0x08, 0, 0, 0x01, 0x42, 0, 0]);
        assert_eq!(call_ss(Menu::Vtsm { vts: 1, title: 1, menu: 3 }, 0), [0x30, 0x08, 0, 0, 0, 0x83, 0, 0]);
    }

    #[test]
    fn system_register_sets() {
        // highlighted button 1: the 6-bit number sits in the top bits of the 16-bit value
        assert_eq!(set_hl_btnn(1), [0x56, 0, 0, 0, 0x04, 0, 0, 0]);
        assert_eq!(set_hl_btnn(36), [0x56, 0, 0, 0, 36 << 2, 0, 0, 0]);
        // audio 1 only, then sub-picture 0x40 (shown, stream 0) and angle 1
        assert_eq!(set_stn(Some(1), None, None), [0x51, 0, 0, 0x81, 0, 0, 0, 0]);
        assert_eq!(set_stn(None, Some(0x40), Some(1)), [0x51, 0, 0, 0, 0xC0, 0x81, 0, 0]);
    }

    #[test]
    fn general_register_set() {
        // the worked example of the DVD VM notes: `mov g1, 10`
        assert_eq!(mov_gprm(1, 10), [0x71, 0x00, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x00]);
        assert_eq!(mov_gprm(15, 0xBEEF), [0x71, 0, 0, 0x0F, 0xBE, 0xEF, 0, 0]);
    }
}
