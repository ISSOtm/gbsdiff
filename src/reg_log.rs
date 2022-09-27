/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::num::ParseIntError;

use discrim::FromDiscriminant;
use parse_display::Display;
use thiserror::Error;

#[derive(Debug)]
pub struct RegLog {
    pub writes: Vec<RegWrite>,
}

impl RegLog {
    pub fn new(writes: Vec<RegWrite>) -> Self {
        Self { writes }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RegWrite {
    pub delta: u32,
    pub reg: Reg,
    pub value: u8,
}

impl RegWrite {
    pub fn try_parse(s: &str) -> Result<Option<Self>, RegWriteParseErr> {
        let (delta, reg_eq_val) = s
            .split_once(char::is_whitespace)
            .ok_or(RegWriteParseErr::NoWhitespace)?;
        // Splitting already trimmed its end
        let delta =
            u32::from_str_radix(delta.trim_start(), 16).map_err(RegWriteParseErr::BadDelta)?;

        let (reg, val) = reg_eq_val
            .split_once('=')
            .ok_or(RegWriteParseErr::BadWrite)?;
        // Splitting already trimmed its start
        let reg = u16::from_str_radix(reg.trim_end(), 16).map_err(RegWriteParseErr::BadReg)?;
        let value = u8::from_str_radix(val.trim(), 16).map_err(RegWriteParseErr::BadValue)?;

        Ok(match Reg::from_discriminant(reg) {
            Ok(reg) => Some(Self { delta, reg, value }),
            Err(_) => None,
        })
    }

    pub fn write_info(&self) -> (Reg, u8) {
        (self.reg, self.value)
    }
}

#[derive(Debug, Error)]
pub enum RegWriteParseErr {
    #[error("Expected delta and write components separated by whitespace")]
    NoWhitespace,
    #[error("Delta is not a valid hex number: {0}")]
    BadDelta(#[source] ParseIntError),
    #[error("Expected write component of the form \"<addr>=<value>\"")]
    BadWrite,
    #[error("Target register is not a valid hex number: {0}")]
    BadReg(#[source] ParseIntError),
    #[error("Written value is not a valid hex number: {0}")]
    BadValue(#[source] ParseIntError),
}

#[derive(Debug, Display, FromDiscriminant, PartialEq, Eq, Clone, Copy)]
#[repr(u16)]
#[display(style = "UPPERCASE")]
pub enum Reg {
    Div = 0xff04,

    Nr10 = 0xff10,
    Nr11 = 0xff11,
    Nr12 = 0xff12,
    Nr13 = 0xff13,
    Nr14 = 0xff14,
    Nr21 = 0xff16,
    Nr22 = 0xff17,
    Nr23 = 0xff18,
    Nr24 = 0xff19,
    Nr30 = 0xff1a,
    Nr31 = 0xff1b,
    Nr32 = 0xff1c,
    Nr33 = 0xff1d,
    Nr34 = 0xff1e,
    Nr41 = 0xff20,
    Nr42 = 0xff21,
    Nr43 = 0xff22,
    Nr44 = 0xff23,
    Nr50 = 0xff24,
    Nr51 = 0xff25,
    Nr52 = 0xff26,

    #[display("wave RAM[0]")]
    Wave0 = 0xff30,
    #[display("wave RAM[1]")]
    Wave1 = 0xff31,
    #[display("wave RAM[2]")]
    Wave2 = 0xff32,
    #[display("wave RAM[3]")]
    Wave3 = 0xff33,
    #[display("wave RAM[4]")]
    Wave4 = 0xff34,
    #[display("wave RAM[5]")]
    Wave5 = 0xff35,
    #[display("wave RAM[6]")]
    Wave6 = 0xff36,
    #[display("wave RAM[7]")]
    Wave7 = 0xff37,
    #[display("wave RAM[8]")]
    Wave8 = 0xff38,
    #[display("wave RAM[9]")]
    Wave9 = 0xff39,
    #[display("wave RAM[10]")]
    WaveA = 0xff3a,
    #[display("wave RAM[11]")]
    WaveB = 0xff3b,
    #[display("wave RAM[12]")]
    WaveC = 0xff3c,
    #[display("wave RAM[13]")]
    WaveD = 0xff3d,
    #[display("wave RAM[14]")]
    WaveE = 0xff3e,
    #[display("wave RAM[15]")]
    WaveF = 0xff3f,
}
