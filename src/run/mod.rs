/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! This module deals with running the CPU simulator for a particular GBS file.

use std::{
    cell::{Cell, RefCell},
    io::Write,
};

use gb_cpu_sim::{
    cpu::{State, TickResult},
    memory::AddressSpace,
};
use parse_display::Display;

use crate::{
    gbs::{AddressKind, Gbs},
    Address, Diagnostic, DiagnosticLevel, Timestamp,
};

mod addr_space;
use addr_space::*;

/// Note: `song_id` is 0-based.
pub(crate) fn simulate_song<T: Write>(
    gbs: &Gbs<'_>,
    song_id: u8,
    max_level: DiagnosticLevel,
    mut timeout: u32,
    allow_timeout: bool,
    silence_timeout: u32,
    watch: Option<(u16, u8)>,
    mut trace_file: Option<T>,
) -> Result<Logbook, Error> {
    let mut logbook = Default::default();
    let logger = RefCell::new(LogbookWriter::new(&mut logbook, max_level));
    let cycles_per_tick: u16 = if gbs.use_timer() {
        (1u16 << gbs.timer_div_bit()) * (256u16 - u16::from(gbs.timer_mod()))
    } else {
        114 * 154 // 114 cycles/scanline times 154 scanlines
    };
    let silence_timer = Cell::new(0);

    if let Some(ref mut trace_file) = trace_file {
        writeln!(trace_file, "==== SONG {} ====", song_id).unwrap_or_else(crate::trace_write_fail);
    }

    // "LOAD" step.
    let mut cpu = State::new(GbsAddrSpace::new(gbs, &logger, &silence_timer));

    // "INIT" step.
    cpu.a = song_id;
    cpu.sp = gbs.stack_ptr();
    cpu.pc = gbs.addr(AddressKind::Init);
    run_func(&mut cpu, trace_file.as_mut(), &logger)?;

    // "PLAY" step.
    loop {
        logger.borrow_mut().next_tick();
        if let Some(ref mut trace_file) = trace_file {
            writeln!(trace_file, "--- TICK {} ---", logger.borrow().tick)
                .unwrap_or_else(crate::trace_write_fail);
        }

        cpu.sp = gbs.stack_ptr();
        cpu.pc = gbs.addr(AddressKind::Play);
        let cycles = run_func(&mut cpu, trace_file.as_mut(), &logger)?;

        if let Some(_diff) = cycles_per_tick.checked_sub(cycles) {
            // TODO: tick DIV etc.
        } else {
            logger.borrow_mut().diagnose(
                DiagnosticLevel::Warning,
                DiagnosticKind::TooLong(cycles, cycles_per_tick),
            );
        }

        // Check termination conditions.
        if silence_timer.get() >= silence_timeout {
            break;
        }
        silence_timer.set(silence_timer.get() + u32::from(cycles_per_tick));
        if let Some((addr, value)) = watch {
            if cpu.read(addr) == value {
                break;
            }
        }
        timeout = match timeout.checked_sub(cycles_per_tick.into()) {
            Some(timeout) => timeout,
            None if allow_timeout => break,
            None => return Err(Error::Timeout),
        };
    }

    Ok(logbook)
}

#[derive(Debug, Default)]
pub(crate) struct Logbook {
    pub diagnostics: Vec<Diagnostic<DiagnosticKind>>,
    pub io_log: Vec<IoAccess>,
}

#[derive(Debug, Display)]
pub(crate) enum DiagnosticKind {
    #[display("unsupported read from ${0:x}")]
    UnsupportedRead(Address),
    #[display("unsupported write of ${1:02x} to ${0:04x}")]
    UnsupportedWrite(Address, u8),
    #[display("read from echo RAM at ${0:x}")]
    EchoRamRead(Address),
    #[display("write of ${0:02x} to echo RAM at ${1:x}")]
    EchoRamWrite(Address, u8),
    #[display("tick took {0} cycles, over the budget of {1} cycles")]
    TooLong(u16, u16),
    #[display("executed a debug opcode at ${0:x}")]
    DebugOp(Address),
}

#[derive(Debug, PartialEq, Eq)]
/// Currently only writes, but reads may also be interesting in the future
pub(crate) struct IoAccess {
    pub when: Timestamp,
    pub pc: Address,
    pub addr: u16,
    pub data: u8,
}

#[derive(Debug, Display)]
/// Errors that immediately stop the execution.
pub(crate) enum Error {
    #[display("executed a `halt` at ${0:x}")]
    Halted(Address),
    #[display("executed a `stop` at ${0:x}")]
    Stopped(Address),
    #[display("executed invalid opcode ${0:02x} at ${1:x}")]
    InvalidOpcode(u8, Address),
    #[display("aborted when SP reached ${0:04x} (expected target: ${1:04x})")]
    PoppedTooDeep(u16, u16),
    #[display("CPU seemingly locked up at ${0:x}")]
    LockedUp(Address),
    #[display("timed out")]
    Timeout,
    #[display("execution has gone haywire: PC = ${0:x}")]
    PcHaywire(Address),
    #[display("stack has gone haywire: SP = ${0:x} (PC = ${1:x})")]
    SpHaywire(Address, Address),
}

/// Run the CPU simulator until a `ret` is executed.
///
/// The function will also return if the pseudo-return-address is popped, or if the stack appears to become less deep than on entry; this is considered an error.
///
/// Note that this function returns *after* the `ret` is executed.
fn run_func<S: AddressSpace, T: Write>(
    cpu: &mut State<S>,
    mut trace_file: Option<T>,
    logger: &RefCell<LogbookWriter>,
) -> Result<u16, Error> {
    let mut total_cycles = 0u16;

    let orig_sp = cpu.sp;
    // SP in ROM does not make sense
    while cpu.sp >= 0x8000 && cpu.sp <= orig_sp {
        let prev_pc = Address(logger.borrow().rom_bank, cpu.pc);
        logger.borrow_mut().pc = cpu.pc;

        // Check that the state is valid
        if (0xFF00..=0xFF7F).contains(&cpu.pc) {
            return Err(Error::PcHaywire(prev_pc));
        }
        if (0xFF00..=0xFF7F).contains(&cpu.sp) {
            return Err(Error::SpHaywire(Address(prev_pc.0, cpu.sp), prev_pc));
        }

        if let Some(ref mut trace_file) = trace_file {
            writeln!(trace_file, "pc=${:04x} b=${:02x} c=${:02x} d=${:02x} e=${:02x} h=${:02x} l=${:02x} a=${:02x} f={}{}{}{} sp=${:04x}",
                cpu.pc, cpu.b, cpu.c, cpu.d, cpu.e, cpu.h, cpu.l, cpu.a,
                if cpu.f.get_z() { "Z" } else {"z"},
                if cpu.f.get_n() { "N" } else {"n"},
                if cpu.f.get_h() { "H" } else {"h"},
                if cpu.f.get_c() { "C" } else {"c"},
                cpu.sp).unwrap_or_else(crate::trace_write_fail);
        }

        match cpu.tick() {
            TickResult::Ok => (), // The easy case, just keep trying
            TickResult::Debug | TickResult::Break => logger.borrow_mut().diagnose(
                DiagnosticLevel::Note,
                DiagnosticKind::DebugOp(prev_pc.clone()),
            ),
            TickResult::Halt => return Err(Error::Halted(prev_pc)),
            TickResult::Stop => return Err(Error::Stopped(prev_pc)),
            TickResult::InvalidOpcode => {
                return Err(Error::InvalidOpcode(cpu.read(prev_pc.1), prev_pc))
            }
        }

        let elapsed = cpu.cycles_elapsed.try_into().unwrap();
        total_cycles = total_cycles
            .checked_add(elapsed)
            .ok_or(Error::LockedUp(prev_pc))?;
        logger.borrow_mut().cycle += elapsed;
        cpu.cycles_elapsed = 0;
    }

    if cpu.sp == orig_sp.wrapping_add(2) {
        Ok(total_cycles)
    } else {
        Err(Error::PoppedTooDeep(cpu.sp, orig_sp))
    }
}

#[derive(Debug)]
struct LogbookWriter<'a> {
    logbook: &'a mut Logbook,
    max_level: DiagnosticLevel,

    rom_bank: u8, // This is the canonical copy, and yes that's ugly af.
    pc: u16,
    tick: u64,
    cycle: u16,
}

impl<'a> LogbookWriter<'a> {
    fn new(logbook: &'a mut Logbook, max_level: DiagnosticLevel) -> Self {
        Self {
            logbook,
            max_level,

            rom_bank: 1,
            pc: 0,
            tick: 0,
            cycle: 0,
        }
    }

    fn next_tick(&mut self) {
        self.tick += 1;
        self.cycle = 0;
    }

    fn now(&self) -> Timestamp {
        Timestamp {
            tick: self.tick,
            cycle: self.cycle,
        }
    }

    fn log(&mut self, addr: u16, data: u8) {
        self.logbook.io_log.push(IoAccess {
            when: self.now(),
            pc: Address(self.rom_bank, self.pc),
            addr,
            data,
        })
    }

    fn diagnose(&mut self, level: DiagnosticLevel, kind: DiagnosticKind) {
        if level <= self.max_level {
            self.logbook.diagnostics.push(Diagnostic {
                when: self.now(),
                pc: Address(self.rom_bank, self.pc),
                level,
                kind,
            });
        }
    }
}
