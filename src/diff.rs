/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::fmt::Display;

use owo_colors::OwoColorize;

use crate::{Reg, RegLog, RegWrite};

// The diffing strategy is to try to split the writes into "frames", which are essentially driver calls.
// Since we only have timing info, we have to rely on a heuristic, i.e. that the time between calls is significantly large.
// The chosen value seems like a reasonable default for drivers running around the Game Boy's refresh rate.
pub const FRAME_BOUNDARY_THRESHOLD: u32 = 0x8000;

pub const WRITE_PEDANTIC_JITTER: u32 = 100;

pub fn diff(
    before_log: &RegLog,
    after_log: &RegLog,
    max_reports: usize,
    be_pedantic: bool,
) -> Vec<Diagnostic> {
    let logs = (&before_log.writes, &after_log.writes);
    let mut indices = (0, 0);

    let mut diagnostics = Vec::new();
    let mut frame = 0;

    while indices.0 != logs.0.len() || indices.1 != logs.1.len() {
        let mut starts = indices;

        // First, reduce the scope by looking for a "frame boundary".
        let find_boundary = |writes: &[RegWrite], idx: &mut usize| {
            while let Some(write) = writes.get(*idx) {
                *idx += 1;
                if write.delta >= FRAME_BOUNDARY_THRESHOLD {
                    break;
                }
            }
        };
        find_boundary(logs.0, &mut indices.0);
        find_boundary(logs.1, &mut indices.1);
        frame += 1;
        let mut cycles = (0, 0);
        let frames = (&logs.0[..indices.0], &logs.1[..indices.1]);

        // Now, we must diff the two frames.
        loop {
            let writes = (&frames.0.get(starts.0), &frames.1.get(starts.1));

            macro_rules! consume {
                ($idx:tt, $write:expr) => {
                    starts.$idx += 1;
                    cycles.$idx += $write.delta;
                };
            }
            macro_rules! diagnose {
                ($level:expr, $which:tt, $kind:expr) => {{
                    use DiagnosticKind::*;
                    use DiagnosticLevel::*;
                    if be_pedantic || ($level) != Pedantic {
                        diagnostics.push(Diagnostic {
                            level: $level,
                            frame,
                            cycles: cycles.$which,
                            kind: $kind,
                        });
                    }
                }};
            }

            match writes {
                // End of (the) line, buddy
                (None, None) => break,

                // Lost your balance?
                (Some(before), None) => {
                    consume!(0, before);
                    diagnose!(Error, 0, Removed(before.write_info()));
                }
                (None, Some(after)) => {
                    consume!(1, after);
                    diagnose!(Error, 1, Added(after.write_info()));
                }

                // Figures this would be the common case, eh?
                (Some(before), Some(after)) => {
                    if before == after {
                        // This is the only easy case.
                        consume!(0, before);
                        consume!(1, after);
                    } else if before.write_info() == after.write_info() {
                        // The write is identical, but has been moved a bit.
                        // Unless the change is large, treat this as a pedantic diagnostic.
                        consume!(0, before);
                        consume!(1, after);
                        diagnose!(
                            if before.delta.abs_diff(after.delta) < WRITE_PEDANTIC_JITTER {
                                Pedantic
                            } else {
                                Error
                            },
                            0,
                            Moved(before.write_info(), (after.delta - before.delta) as i32)
                        );
                    }
                    // Oh god. Welcome to half-assed heuristics, please do not judge me :(
                    else if before.reg == after.reg {
                        // The target register is identical, but the value being written is not.
                        // Let's assume they are the same write, except bugged.
                        consume!(0, before);
                        consume!(1, after);
                        diagnose!(Error, 0, OtherValue(before.write_info(), after.value));
                    } else if before.value == after.value {
                        // The written value is identical, but the target register is not.
                        // This is much more iffy than the above, but can stem from e.g. a typo.
                        consume!(0, before);
                        consume!(1, after);
                        diagnose!(Error, 0, OtherReg(before.write_info(), after.reg));
                    } else {
                        // Nothing matches. Let's report the earliest one of the two.
                        let putative_cycles = (cycles.0 + before.delta, cycles.1 + after.delta);
                        if putative_cycles.0 < putative_cycles.1 {
                            consume!(0, before);
                            diagnose!(Error, 0, Removed(before.write_info()));
                        } else {
                            consume!(1, after);
                            diagnose!(Error, 1, Added(after.write_info()));
                        }
                    }
                }
            }

            if diagnostics.len() >= max_reports {
                break;
            }
        }
    }

    diagnostics
}

#[derive(Debug)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub frame: u32,
    pub cycles: u32,
    pub kind: DiagnosticKind,
}

#[derive(Debug)]
pub enum DiagnosticKind {
    /// Present before, but not after.
    Removed((Reg, u8)),
    /// Present after, but not before.
    Added((Reg, u8)),
    /// A few cycles apart.
    Moved((Reg, u8), i32),
    /// Same reg, different values.
    OtherValue((Reg, u8), u8),
    /// Same value, different reg.
    OtherReg((Reg, u8), Reg),
}

impl Display for DiagnosticKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Removed((reg, value)) => write!(f, "Missing write of ${:02x} to {}", value, reg),
            Self::Added((reg, value)) => write!(f, "New write of ${:02x} to {}", value, reg),
            Self::Moved((reg, value), delta) => write!(
                f,
                "Write of ${:02x} to {} occurs {} cycles {}",
                value,
                reg,
                delta.abs(),
                if *delta < 0 { "earlier" } else { "later" }
            ),
            Self::OtherValue((reg, before), after) => write!(
                f,
                "Write of ${:02x} to {} now writes ${:02x}",
                before, reg, after
            ),
            Self::OtherReg((before, value), after) => write!(
                f,
                "Write of ${:02x} to {} is written to {} instead",
                value, before, after
            ),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Pedantic,
    Error,
}

impl Display for DiagnosticLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pedantic => write!(f, "{}", "Note".bright_blue()),
            Self::Error => write!(f, "{}", "Error".bright_red()),
        }
    }
}
