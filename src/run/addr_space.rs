use std::cell::{Cell, RefCell};

use gb_cpu_sim::{memory::AddressSpace, reg::HwReg};

use crate::{
    gbs::{AddressKind, Gbs},
    Address,
};

use super::{DiagnosticKind, DiagnosticLevel, LogbookWriter};

#[derive(Debug)]
pub struct GbsAddrSpace<'a> {
    rom: &'a [u8],
    load_addr: u16,

    sram: [u8; 0x2000],
    wram: [u8; 0x2000],
    hram: [u8; 0x7F],

    apu: Apu<'a>,

    logger: &'a RefCell<LogbookWriter<'a>>,
}

impl<'a> GbsAddrSpace<'a> {
    pub(super) fn new(
        gbs: &'a Gbs<'_>,
        logger: &'a RefCell<LogbookWriter<'a>>,
        silence_timer: &'a Cell<u32>,
    ) -> Self {
        let rom = gbs.rom();
        let load_addr = gbs.addr(AddressKind::Load);

        Self {
            rom,
            load_addr,

            sram: [0; 0x2000],
            wram: [0; 0x2000],
            hram: [0; 0x7F],

            apu: Apu::new(logger, silence_timer),

            logger,
        }
    }

    fn diagnose(&self, level: DiagnosticLevel, kind: DiagnosticKind) {
        self.logger.borrow_mut().diagnose(level, kind);
    }

    fn cur_bank_addr(&self, addr: u16) -> Address {
        Address(self.logger.borrow().rom_bank, addr)
    }
}

impl AddressSpace for GbsAddrSpace<'_> {
    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3FFF => {
                // If the address is in the loaded area, output it; otherwise, fall back to $FF
                // (Note: this should eventually resolve to a jump to $0038 via rst $38.)
                address
                    .checked_sub(self.load_addr)
                    .and_then(|ofs| self.rom.get(usize::from(ofs)).copied())
                    .unwrap_or_else(|| {
                        self.diagnose(
                            DiagnosticLevel::Warning,
                            DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                        );
                        0xFF
                    })
            }
            0x4000..=0x7FFF => {
                let rom_bank = self.logger.borrow().rom_bank;
                (usize::from(address - 0x4000) + usize::from(rom_bank) * 0x4000)
                    .checked_sub(self.load_addr.into())
                    .and_then(|ofs| self.rom.get(ofs).copied())
                    .unwrap_or_else(|| {
                        self.diagnose(
                            DiagnosticLevel::Warning,
                            DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                        );
                        0x00 // The spec says "should"...
                    })
            }
            0x8000..=0x9FFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }
            0xA000..=0xBFFF => self.sram[usize::from(address - 0xA000)],
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)],
            0xE000..=0xFDFF => {
                self.diagnose(
                    DiagnosticLevel::Note,
                    DiagnosticKind::EchoRamRead(self.cur_bank_addr(address)),
                );
                self.wram[usize::from(address - 0xE000)]
            }
            0xFE00..=0xFEFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }
            0xFF00..=0xFF7F => self.apu.read(address).unwrap_or_else(|| {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }),
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)],
            0xFFFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        match address {
            0x2000..=0x3FFF => {
                self.logger.borrow_mut().rom_bank = data;
                if data == 0 {
                    self.diagnose(
                        DiagnosticLevel::Warning,
                        DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                    );
                }
            }
            0x0000..=0x7FFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                );
            }
            0x8000..=0x9FFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                );
            }
            0xA000..=0xBFFF => self.sram[usize::from(address - 0xA000)] = data,
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)] = data,
            0xE000..=0xFDFF => {
                self.diagnose(
                    DiagnosticLevel::Note,
                    DiagnosticKind::EchoRamWrite(self.cur_bank_addr(address), data),
                );
                self.wram[usize::from(address - 0xE000)] = data
            }
            0xFE00..=0xFEFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                );
            }
            0xFF00..=0xFF7F => self.apu.write(address, data).unwrap_or_else(|| {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                )
            }),
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)] = data,
            0xFFFF => {
                self.diagnose(
                    DiagnosticLevel::Warning,
                    DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
                );
            }
        }
    }
}

#[derive(Debug)]
/// The APU as modelled by the GBS spec.
struct Apu<'a> {
    nr10: u8,
    nr11: u8,
    nr12: u8,
    nr13: u8,
    nr14: u8,

    nr21: u8,
    nr22: u8,
    nr23: u8,
    nr24: u8,

    nr30: u8,
    nr31: u8,
    nr32: u8,
    nr33: u8,
    nr34: u8,

    nr41: u8,
    nr42: u8,
    nr43: u8,
    nr44: u8,

    nr50: u8,
    nr51: u8,
    nr52: u8,

    wave_ram: [u8; 16],

    silence_timer: &'a Cell<u32>,
    logger: &'a RefCell<LogbookWriter<'a>>,
}

impl<'a> Apu<'a> {
    fn new(logger: &'a RefCell<LogbookWriter<'a>>, silence_timer: &'a Cell<u32>) -> Self {
        Self {
            nr10: 0,
            nr11: 0,
            nr12: 0,
            nr13: 0,
            nr14: 0,
            nr21: 0,
            nr22: 0,
            nr23: 0,
            nr24: 0,
            nr30: 0,
            nr31: 0,
            nr32: 0,
            nr33: 0,
            nr34: 0,
            nr41: 0,
            nr42: 0,
            nr43: 0,
            nr44: 0,
            nr50: 0,
            nr51: 0,
            nr52: 0,
            wave_ram: Default::default(),
            silence_timer,
            logger,
        }
    }

    fn diagnose(&self, level: DiagnosticLevel, kind: DiagnosticKind) {
        self.logger.borrow_mut().diagnose(level, kind);
    }

    fn log(&self, addr: u16, data: u8) {
        self.logger.borrow_mut().log(addr, data);
    }

    fn cur_bank_addr(&self, addr: u16) -> Address {
        Address(self.logger.borrow().rom_bank, addr)
    }

    fn read(&self, address: u16) -> Option<u8> {
        Some(match HwReg::try_from(address) {
            Ok(HwReg::Nr10) => self.nr10 | 0x80,
            Ok(HwReg::Nr11) => self.nr11 | 0x3F,
            Ok(HwReg::Nr12) => self.nr12,
            Ok(HwReg::Nr13) => 0xFF,
            Ok(HwReg::Nr14) => self.nr14 | 0xBF,

            Err(0xFF15) => {
                self.diagnose(
                    DiagnosticLevel::Note,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }
            Ok(HwReg::Nr21) => self.nr21 | 0x3F,
            Ok(HwReg::Nr22) => self.nr22,
            Ok(HwReg::Nr23) => 0xFF,
            Ok(HwReg::Nr24) => self.nr24 | 0xBF,

            Ok(HwReg::Nr30) => self.nr30 | 0x7F,
            Ok(HwReg::Nr31) => self.nr31,
            Ok(HwReg::Nr32) => self.nr32,
            Ok(HwReg::Nr33) => 0xFF,
            Ok(HwReg::Nr34) => self.nr34 | 0xBF,

            Err(0xFF1F) => {
                self.diagnose(
                    DiagnosticLevel::Note,
                    DiagnosticKind::UnsupportedRead(self.cur_bank_addr(address)),
                );
                0xFF
            }
            Ok(HwReg::Nr41) => self.nr41 | 0xC0,
            Ok(HwReg::Nr42) => self.nr42,
            Ok(HwReg::Nr43) => 0xFF,
            Ok(HwReg::Nr44) => self.nr44 | 0xBF,

            Ok(HwReg::Nr50) => self.nr50,
            Ok(HwReg::Nr51) => self.nr51,
            Ok(HwReg::Nr52) => self.nr52 | 0x70,

            Ok(
                HwReg::Wave0
                | HwReg::Wave1
                | HwReg::Wave2
                | HwReg::Wave3
                | HwReg::Wave4
                | HwReg::Wave5
                | HwReg::Wave6
                | HwReg::Wave7
                | HwReg::Wave8
                | HwReg::Wave9
                | HwReg::WaveA
                | HwReg::WaveB
                | HwReg::WaveC
                | HwReg::WaveD
                | HwReg::WaveE
                | HwReg::WaveF,
            ) => self.wave_ram[usize::from(address - 0xFF30)], // TODO: implement wave RAM locking

            _ => return None,
        })
    }

    fn write(&mut self, address: u16, data: u8) -> Option<()> {
        self.log(address, data);

        // TODO: the APU is currently never ticked. Any reads back may be wrong...

        match HwReg::try_from(address) {
            Ok(HwReg::Nr10) => self.nr10 = data,
            Ok(HwReg::Nr11) => self.nr11 = data,
            Ok(HwReg::Nr12) => self.nr12 = data,
            Ok(HwReg::Nr13) => self.nr13 = data,
            Ok(HwReg::Nr14) => self.nr14 = data,
            Err(0xFF15) => self.diagnose(
                DiagnosticLevel::Note,
                DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
            ),

            Ok(HwReg::Nr21) => self.nr21 = data,
            Ok(HwReg::Nr22) => self.nr22 = data,
            Ok(HwReg::Nr23) => self.nr23 = data,
            Ok(HwReg::Nr24) => self.nr24 = data,

            Ok(HwReg::Nr30) => self.nr30 = data,
            Ok(HwReg::Nr31) => self.nr31 = data,
            Ok(HwReg::Nr32) => self.nr32 = data,
            Ok(HwReg::Nr33) => self.nr33 = data,
            Ok(HwReg::Nr34) => self.nr34 = data,

            Err(0xFF1F) => self.diagnose(
                DiagnosticLevel::Note,
                DiagnosticKind::UnsupportedWrite(self.cur_bank_addr(address), data),
            ),
            Ok(HwReg::Nr41) => self.nr41 = data,
            Ok(HwReg::Nr42) => self.nr42 = data,
            Ok(HwReg::Nr43) => self.nr43 = data,
            Ok(HwReg::Nr44) => self.nr44 = data,

            Ok(HwReg::Nr50) => self.nr50 = data,
            Ok(HwReg::Nr51) => self.nr51 = data,
            Ok(HwReg::Nr52) => self.nr52 = data,

            Ok(
                HwReg::Wave0
                | HwReg::Wave1
                | HwReg::Wave2
                | HwReg::Wave3
                | HwReg::Wave4
                | HwReg::Wave5
                | HwReg::Wave6
                | HwReg::Wave7
                | HwReg::Wave8
                | HwReg::Wave9
                | HwReg::WaveA
                | HwReg::WaveB
                | HwReg::WaveC
                | HwReg::WaveD
                | HwReg::WaveE
                | HwReg::WaveF,
            ) => self.wave_ram[usize::from(address - 0xFF30)] = data, // TODO: implement wave RAM locking

            _ => return None,
        };

        self.silence_timer.set(0);
        Some(())
    }
}
