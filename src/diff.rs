/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{cmp::Ordering, fmt::Display};

use gb_cpu_sim::reg::HwReg;

use crate::{run::IoAccess, Diagnostic, DiagnosticLevel};

#[derive(Debug)]
pub struct DiffGenerator<'a> {
    // Parameters
    logs: (&'a [IoAccess], &'a [IoAccess]),
    jitter: u16,

    // State
    indices: (usize, usize),
}

impl<'a> DiffGenerator<'a> {
    pub(crate) fn new(before_log: &'a [IoAccess], after_log: &'a [IoAccess], jitter: u16) -> Self {
        Self {
            logs: (before_log, after_log),
            jitter,
            indices: (0, 0),
        }
    }
}

impl Iterator for DiffGenerator<'_> {
    type Item = Diagnostic<DiagnosticKind>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Only a single code path loops back.
            return match (
                self.logs.0.get(self.indices.0),
                self.logs.1.get(self.indices.1),
            ) {
                (None, None) => None, // We're done!

                (Some(before), None) => {
                    self.indices.0 += 1;
                    diagnose(
                        before,
                        DiagnosticLevel::Error,
                        DiagnosticKind::Removed(before.addr, before.data),
                    )
                }
                (None, Some(after)) => {
                    self.indices.1 += 1;
                    diagnose(
                        after,
                        DiagnosticLevel::Error,
                        DiagnosticKind::Added(after.addr, after.data),
                    )
                }

                (Some(before), Some(after)) => {
                    // If both belong to the same tick, we can feasibly compare them.
                    // Otherwise, mimic the logic above.
                    match before.when.tick.cmp(&after.when.tick) {
                        Ordering::Less => {
                            self.indices.0 += 1;
                            return diagnose(
                                before,
                                DiagnosticLevel::Error,
                                DiagnosticKind::Removed(before.addr, before.data),
                            );
                        }
                        Ordering::Greater => {
                            self.indices.1 += 1;
                            return diagnose(
                                after,
                                DiagnosticLevel::Error,
                                DiagnosticKind::Added(after.addr, after.data),
                            );
                        }
                        Ordering::Equal => (),
                    }

                    // If the two match exactly, we have nothing to report; try again.
                    // This is the only easy case.
                    if before == after {
                        self.indices.0 += 1;
                        self.indices.1 += 1;
                        continue;
                    }

                    // So there is a difference: it can be timing, address, or data.
                    // Timing being the most sensitive, it will not be used as a triaging criterion.
                    match (before.addr == after.addr, before.data == after.data) {
                        (true, true) => {
                            // The write is identical, but has been moved a bit.
                            self.indices.0 += 1;
                            self.indices.1 += 1;
                            diagnose(
                                after,
                                if before.when.cycle.abs_diff(after.when.cycle) < self.jitter {
                                    DiagnosticLevel::Note
                                } else {
                                    DiagnosticLevel::Error
                                },
                                DiagnosticKind::Moved(
                                    before.addr,
                                    before.data,
                                    (after.when.cycle as i32)
                                        .wrapping_sub(before.when.cycle as i32),
                                ),
                            )
                        }
                        // Oh god. Welcome to half-assed heuristics, please do not judge me :(
                        (true, false) => {
                            // The target register is identical, but the value being written is not.
                            // Let's assume they are the same write, except bugged.
                            self.indices.0 += 1;
                            self.indices.1 += 1;
                            diagnose(
                                after,
                                DiagnosticLevel::Error,
                                DiagnosticKind::OtherValue(before.addr, before.data, after.data),
                            )
                        }
                        (false, true) => {
                            // The written value is identical, but the target register is not.
                            // This is much more iffy than the above, but can stem from e.g. a typo.
                            self.indices.0 += 1;
                            self.indices.1 += 1;
                            diagnose(
                                after,
                                DiagnosticLevel::Error,
                                DiagnosticKind::OtherReg(before.addr, before.data, after.addr),
                            )
                        }
                        (false, false) => {
                            // Nothing matches.
                            // Let's compare one beyond; if the address matches with the opposite "N+1", assume that they're meant to be paired.
                            // (The value is too volatile, so it's not checked here.)
                            match (
                                self.logs.0.get(self.indices.0 + 1),
                                self.logs.1.get(self.indices.1 + 1),
                            ) {
                                (Some(before2), _) if before2.addr == after.addr => {
                                    self.indices.0 += 1;
                                    diagnose(
                                        before,
                                        DiagnosticLevel::Error,
                                        DiagnosticKind::Removed(before.addr, before.data),
                                    )
                                }
                                (_, Some(after2)) if before.addr == after2.addr => {
                                    self.indices.1 += 1;
                                    diagnose(
                                        after,
                                        DiagnosticLevel::Error,
                                        DiagnosticKind::Added(after.addr, after.data),
                                    )
                                }
                                _ => {
                                    // Let's report the earliest one of the two.
                                    if before.when.cycle < after.when.cycle {
                                        self.indices.0 += 1;
                                        diagnose(
                                            before,
                                            DiagnosticLevel::Error,
                                            DiagnosticKind::Removed(before.addr, before.data),
                                        )
                                    } else {
                                        self.indices.1 += 1;
                                        diagnose(
                                            after,
                                            DiagnosticLevel::Error,
                                            DiagnosticKind::Added(after.addr, after.data),
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            };
        }
    }
}

fn diagnose(
    access: &IoAccess,
    level: DiagnosticLevel,
    kind: DiagnosticKind,
) -> Option<Diagnostic<DiagnosticKind>> {
    Some(Diagnostic {
        when: access.when.clone(),
        pc: access.pc.clone(),
        level,
        kind,
    })
}

#[derive(Debug)]
pub enum DiagnosticKind {
    /// Present before, but not after.
    Removed(u16, u8),
    /// Present after, but not before.
    Added(u16, u8),
    /// A few cycles apart.
    Moved(u16, u8, i32),
    /// Same reg, different values.
    OtherValue(u16, u8, u8),
    /// Same value, different reg.
    OtherReg(u16, u8, u16),
}

impl Display for DiagnosticKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Removed(reg, value) => {
                write!(f, "Missing write of ${:02x} to {}", value, RegDispl(*reg))
            }
            Self::Added(reg, value) => {
                write!(f, "New write of ${:02x} to {}", value, RegDispl(*reg))
            }
            Self::Moved(reg, value, delta) => write!(
                f,
                "Wrote ${:02x} to {} {} cycles {}",
                value,
                RegDispl(*reg),
                delta.abs(),
                if *delta < 0 { "earlier" } else { "later" }
            ),
            Self::OtherValue(reg, before, after) => write!(
                f,
                "Wrote ${:02x} to {} instead of ${:02x}",
                after,
                RegDispl(*reg),
                before,
            ),
            Self::OtherReg(before, value, after) => write!(
                f,
                "${:02x} is written to {} instead of {}",
                value,
                RegDispl(*after),
                RegDispl(*before),
            ),
        }
    }
}

struct RegDispl(u16);

impl Display for RegDispl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match HwReg::try_from(self.0) {
            Ok(HwReg::Ramg) => write!(f, "RAMG"),
            Ok(HwReg::Romb0) => write!(f, "ROMB0"),
            Ok(HwReg::Romb1) => write!(f, "ROMB1"),
            Ok(HwReg::Ramb) => write!(f, "RAMB"),
            Ok(HwReg::Rtclatch) => write!(f, "RTCLATCH"),
            Ok(HwReg::P1) => write!(f, "P1"),
            Ok(HwReg::Sb) => write!(f, "SB"),
            Ok(HwReg::Sc) => write!(f, "SC"),
            Ok(HwReg::Div) => write!(f, "DIV"),
            Ok(HwReg::Tima) => write!(f, "TIMA"),
            Ok(HwReg::Tma) => write!(f, "TMA"),
            Ok(HwReg::Tac) => write!(f, "TAC"),
            Ok(HwReg::If) => write!(f, "IF"),
            Ok(HwReg::Nr10) => write!(f, "NR10"),
            Ok(HwReg::Nr11) => write!(f, "NR11"),
            Ok(HwReg::Nr12) => write!(f, "NR12"),
            Ok(HwReg::Nr13) => write!(f, "NR13"),
            Ok(HwReg::Nr14) => write!(f, "NR14"),
            Ok(HwReg::Nr21) => write!(f, "NR21"),
            Ok(HwReg::Nr22) => write!(f, "NR22"),
            Ok(HwReg::Nr23) => write!(f, "NR23"),
            Ok(HwReg::Nr24) => write!(f, "NR24"),
            Ok(HwReg::Nr30) => write!(f, "NR30"),
            Ok(HwReg::Nr31) => write!(f, "NR31"),
            Ok(HwReg::Nr32) => write!(f, "NR32"),
            Ok(HwReg::Nr33) => write!(f, "NR33"),
            Ok(HwReg::Nr34) => write!(f, "NR34"),
            Ok(HwReg::Nr41) => write!(f, "NR41"),
            Ok(HwReg::Nr42) => write!(f, "NR42"),
            Ok(HwReg::Nr43) => write!(f, "NR43"),
            Ok(HwReg::Nr44) => write!(f, "NR44"),
            Ok(HwReg::Nr50) => write!(f, "NR50"),
            Ok(HwReg::Nr51) => write!(f, "NR51"),
            Ok(HwReg::Nr52) => write!(f, "NR52"),
            Ok(HwReg::Wave0) => write!(f, "Wave RAM[0]"),
            Ok(HwReg::Wave1) => write!(f, "Wave RAM[1]"),
            Ok(HwReg::Wave2) => write!(f, "Wave RAM[2]"),
            Ok(HwReg::Wave3) => write!(f, "Wave RAM[3]"),
            Ok(HwReg::Wave4) => write!(f, "Wave RAM[4]"),
            Ok(HwReg::Wave5) => write!(f, "Wave RAM[5]"),
            Ok(HwReg::Wave6) => write!(f, "Wave RAM[6]"),
            Ok(HwReg::Wave7) => write!(f, "Wave RAM[7]"),
            Ok(HwReg::Wave8) => write!(f, "Wave RAM[8]"),
            Ok(HwReg::Wave9) => write!(f, "Wave RAM[9]"),
            Ok(HwReg::WaveA) => write!(f, "Wave RAM[10]"),
            Ok(HwReg::WaveB) => write!(f, "Wave RAM[11]"),
            Ok(HwReg::WaveC) => write!(f, "Wave RAM[12]"),
            Ok(HwReg::WaveD) => write!(f, "Wave RAM[13]"),
            Ok(HwReg::WaveE) => write!(f, "Wave RAM[14]"),
            Ok(HwReg::WaveF) => write!(f, "Wave RAM[15]"),
            Ok(HwReg::Lcdc) => write!(f, "LCDC"),
            Ok(HwReg::Stat) => write!(f, "STAT"),
            Ok(HwReg::Scy) => write!(f, "SCY"),
            Ok(HwReg::Scx) => write!(f, "SCX"),
            Ok(HwReg::Ly) => write!(f, "LY"),
            Ok(HwReg::Lyc) => write!(f, "LYC"),
            Ok(HwReg::Dma) => write!(f, "DMA"),
            Ok(HwReg::Bgp) => write!(f, "BGP"),
            Ok(HwReg::Obp0) => write!(f, "OBP0"),
            Ok(HwReg::Obp1) => write!(f, "OBP1"),
            Ok(HwReg::Wy) => write!(f, "WY"),
            Ok(HwReg::Wx) => write!(f, "WX"),
            Ok(HwReg::Key1) => write!(f, "KEY1"),
            Ok(HwReg::Vbk) => write!(f, "VBK"),
            Ok(HwReg::Hdma1) => write!(f, "HDMA1"),
            Ok(HwReg::Hdma2) => write!(f, "HDMA2"),
            Ok(HwReg::Hdma3) => write!(f, "HDMA3"),
            Ok(HwReg::Hdma4) => write!(f, "HDMA4"),
            Ok(HwReg::Hdma5) => write!(f, "HDMA5"),
            Ok(HwReg::Rp) => write!(f, "RP"),
            Ok(HwReg::Bcps) => write!(f, "BCPS"),
            Ok(HwReg::Bcpd) => write!(f, "BCPD"),
            Ok(HwReg::Ocps) => write!(f, "OCPS"),
            Ok(HwReg::Ocpd) => write!(f, "OCPD"),
            Ok(HwReg::Svbk) => write!(f, "SVBK"),
            Ok(HwReg::Pcm12) => write!(f, "PCM12"),
            Ok(HwReg::Pcm34) => write!(f, "PCM34"),
            Ok(HwReg::Ie) => write!(f, "IE"),
            Ok(reg) => write!(f, "{}", u16::from(reg)),
            Err(addr) => write!(f, "${addr:04x}"),
        }
    }
}
