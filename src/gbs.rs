/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! This module deals with parsing GBS files.

use std::hint::unreachable_unchecked;

use parse_display::Display;

#[derive(Debug)]
pub struct Gbs<'gbs>(&'gbs [u8]);

impl<'gbs> Gbs<'gbs> {
    const HEADER_LEN: usize = 0x70;
    /// The driver should never access this area.
    pub const MIN_ROM_ADDR: u16 = 0x400;

    pub fn new(data: &'gbs [u8]) -> Result<Self, FormatError<'gbs>> {
        if data.len() < Self::HEADER_LEN {
            return Err(FormatError::TruncatedHeader(data.len()));
        }

        let magic = &data[0..3];
        if magic != b"GBS" {
            return Err(FormatError::BadMagic(magic));
        }

        let version = data[3];
        if version != 1 {
            return Err(FormatError::UnsupportedVersion(version));
        }

        let gbs = Self(data);
        if gbs.nb_songs() == 0 {
            return Err(FormatError::ZeroSongs);
        }

        let load_addr = gbs.addr(AddressKind::Load);
        if !(Self::MIN_ROM_ADDR..=0x4000).contains(&load_addr) {
            return Err(FormatError::BadAddress(AddressKind::Load, load_addr));
        }
        for kind in [AddressKind::Init, AddressKind::Play] {
            let addr = gbs.addr(kind);
            if !(load_addr..0x8000).contains(&addr) {
                return Err(FormatError::BadAddress(kind, addr));
            }
        }

        Ok(gbs)
    }

    fn read16(&self, ofs: usize) -> u16 {
        let raw = [self.0[ofs], self.0[ofs + 1]];
        u16::from_le_bytes(raw)
    }

    pub fn nb_songs(&self) -> u8 {
        self.0[4]
    }

    pub fn first_song(&self) -> u8 {
        self.0[5]
    }

    pub fn addr(&self, kind: AddressKind) -> u16 {
        self.read16(kind.ofs())
    }

    pub fn stack_ptr(&self) -> u16 {
        self.read16(12)
    }

    pub fn timer_mod(&self) -> u8 {
        self.0[14]
    }

    pub fn timer_div_bit(&self) -> u8 {
        match self.timer_ctrl() & 3 {
            0 => 9,
            1 => 3,
            2 => 5,
            3 => 7,
            _ => unsafe { unreachable_unchecked() },
        }
    }

    pub fn use_timer(&self) -> bool {
        self.timer_ctrl() & 4 != 0
    }

    pub fn double_speed(&self) -> bool {
        self.timer_ctrl() & 0x80 != 0
    }

    fn timer_ctrl(&self) -> u8 {
        self.0[15]
    }

    pub fn rom(&self) -> &[u8] {
        &self.0[0x70..]
    }
}

#[derive(Debug, Display)]
pub enum FormatError<'a> {
    #[display("expected at least 0x70 header bytes, got only {0}")]
    TruncatedHeader(usize),
    #[display("expected \"GBS\" magic, got \"{0:?}\"")]
    BadMagic(&'a [u8]),
    #[display("unsupported version {0}")]
    UnsupportedVersion(u8),
    #[display("zero songs specified")]
    ZeroSongs,
    #[display("bad {0} address ${1:04x}")]
    BadAddress(AddressKind, u16),
}

#[derive(Debug, Display, Clone, Copy)]
#[display(style = "lowercase")]
pub enum AddressKind {
    Load,
    Init,
    Play,
}

impl AddressKind {
    fn ofs(&self) -> usize {
        match self {
            Self::Load => 6,
            Self::Init => 8,
            Self::Play => 10,
        }
    }
}
