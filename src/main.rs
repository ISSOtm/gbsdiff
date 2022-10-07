/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    cmp::Ordering,
    fmt::{Display, LowerHex},
    fs::{self, File},
    io,
    str::FromStr,
};

use argh::FromArgs;
use owo_colors::OwoColorize;
use slicedisplay::SliceDisplay;

mod diff;
mod gbs;
use gbs::Gbs;
mod run;

const CYCLES_PER_SEC: u32 = 1048576;

#[derive(FromArgs)]
/// Analyze differences in audio register writes between two GBS files.
struct Args {
    #[argh(option, short = 'l', default = "DiagnosticLevel::Warning")]
    /// silence diagnostics with a higher level than this (default: warning)
    max_level: DiagnosticLevel,
    #[argh(option, short = 'm', default = "1000")]
    /// how many diagnostics to report per song, at most (default: 1000)
    max_reports: usize,
    #[argh(option, short = 't', default = "60")]
    /// time out simulation of a song after this many seconds (default: 60)
    timeout: u8,
    #[argh(switch, short = 'T')]
    /// make timeout non-fatal (useful for looping tracks)
    allow_timeout: bool,
    #[argh(option, short = 's', default = "4")]
    /// consider that a song ended after this many seconds of silence (default: 4)
    slience_timeout: u8,
    #[argh(option, short = 'w', from_str_fn(parse_watch_arg))]
    /// consider that a song ended when `ADDR=VALUE` (both hex numbers)
    watch: Option<(u16, u8)>,
    #[argh(option)]
    /// log CPU activity to this file (significant slowdown)
    trace: Option<String>,
    #[argh(option, short = 'd', default = "BeforeOrAfter::After")]
    /// print the diagnostics of either the "before" GBS, the "after" one, or "none" (default: after)
    print_diagnostics: BeforeOrAfter,

    #[argh(positional)]
    /// path to the GBS file that was built before the changes
    before: String,
    #[argh(positional)]
    /// path to the GBS file that was built after the changes
    after: String,
}
fn main() {
    let args: Args = argh::from_env();
    let timeout = u32::from(args.timeout) * CYCLES_PER_SEC;
    let silence_timeout = u32::from(args.slience_timeout) * CYCLES_PER_SEC;
    let mut trace_file = args.trace.map(|path| {
        File::create(path).unwrap_or_else(|err| {
            eprintln!("Failed to open trace file: {}", err);
            std::process::exit(2);
        })
    });

    let read_file = |path| {
        eprintln!(
            "{} {} {}...",
            "==>".bold(),
            "Reading".bright_cyan().bold(),
            &path
        );

        fs::read(&path).unwrap_or_else(|err| {
            eprintln!(
                "{} while reading {}: {}",
                "Error".bright_red().bold(),
                path,
                err
            );
            std::process::exit(2);
        })
    };
    let parse_gbs = |data, path| {
        Gbs::new(data).unwrap_or_else(|err| {
            eprintln!("{} parsing {}: {}", "Error".bright_red().bold(), path, err);
            std::process::exit(2);
        })
    };
    let before_data = read_file(&args.before);
    let before_gbs = parse_gbs(&before_data, &args.before);
    let after_data = read_file(&args.after);
    let after_gbs = parse_gbs(&after_data, &args.after);

    let nb_songs = std::cmp::min(before_gbs.nb_songs(), after_gbs.nb_songs());
    if before_gbs.nb_songs() != after_gbs.nb_songs() {
        eprintln!(
            "{}: Earlier GBS has {} songs, later has {}; only comparing first {}",
            "warning".bright_yellow().bold(),
            before_gbs.nb_songs(),
            after_gbs.nb_songs(),
            nb_songs,
        );
    }

    let mut failed = Vec::new();
    for i in 0..nb_songs {
        let song_ids = (i + before_gbs.first_song(), i + after_gbs.first_song());

        eprintln!(
            "{} {} songs {}...",
            "==>".bold(),
            "Simulating".bright_cyan().bold(),
            SongIDs(song_ids),
        );
        macro_rules! simulate {
            ($gbs:expr, $song_id:expr, $path:expr) => {
                match run::simulate_song(
                    $gbs,
                    $song_id,
                    args.max_level,
                    timeout,
                    args.allow_timeout,
                    silence_timeout,
                    args.watch,
                    trace_file.as_mut(),
                ) {
                    Ok(log) => log,
                    Err(err) => {
                        eprintln!(
                            "{} to simulate {} song #{}: {}",
                            "Failed".bold().bright_red(),
                            $path,
                            $song_id,
                            err
                        );
                        failed.push(SongIDs(song_ids));
                        continue;
                    }
                }
            };
        }
        let logs = (
            simulate!(&before_gbs, song_ids.0, args.before),
            simulate!(&after_gbs, song_ids.1, args.after),
        );

        eprintln!(
            "{} {} songs {}...",
            "==>".bold(),
            "Comparing".bright_cyan().bold(),
            SongIDs(song_ids),
        );

        let mut ok = true;
        let mut tick = u64::MAX;
        let mut diagnostics = match args.print_diagnostics {
            BeforeOrAfter::Before => Some(&logs.0),
            BeforeOrAfter::After => Some(&logs.1),
            BeforeOrAfter::None => None,
        }
        .map(|logs| logs.diagnostics.iter().peekable());

        let print_tick = |tick| eprintln!("{} Tick {} {}", "====".bold(), tick, "====".bold());
        let mut i = 0;
        macro_rules! report {
            ($diag:expr $(, $label:tt)?) => {
                eprintln!(
                    "{} on cycle {} (PC = ${:04x}): {}",
                    $diag.level, $diag.when.cycle, $diag.pc, $diag.kind
                );
                i += 1;
                if i == args.max_reports {
                    eprintln!(
                        "...stopping at {} diagnostics. Go fix your code!",
                        args.max_reports
                    );
                    break $($label)?;
                }
            };
        }

        'report: for diagnostic in diff::DiffGenerator::new(&logs.0.io_log, &logs.1.io_log)
            .filter(|diag| diag.level <= args.max_level)
        {
            ok = false;

            if diagnostic.when.tick != tick {
                if let Some(diagnostics) = diagnostics.as_mut() {
                    while let Some(diag) = diagnostics.peek() {
                        match tick.cmp(&diag.when.tick) {
                            Ordering::Greater => break, // Don't print diagnostics for upcoming ticks quite yet
                            Ordering::Less => {
                                tick = diag.when.tick;
                                print_tick(tick);
                            }
                            Ordering::Equal => (),
                        }

                        report!(diag, 'report);

                        diagnostics.next();
                    }
                }

                if tick != diagnostic.when.tick {
                    tick = diagnostic.when.tick;
                    print_tick(tick);
                }
            }

            report!(diagnostic);
        }

        // Print any leftover diagnostics
        if let Some(diagnostics) = diagnostics.as_mut() {
            for diag in diagnostics {
                if tick != diag.when.tick {
                    tick = diag.when.tick;
                    print_tick(tick);
                }
                report!(diag);
            }
        }

        if ok {
            eprintln!("{}", "OK!".bright_green().bold());
        } else {
            failed.push(SongIDs(song_ids));
        }
    }

    if failed.is_empty() {
        eprintln!(
            "{} {}",
            "==>".bold(),
            "All songs are OK!".bright_green().bold()
        );
    } else if failed.len() == 1 {
        eprintln!("{} song: {}", "Failing".bright_red().bold(), failed[0]);
        std::process::exit(1);
    } else {
        eprintln!(
            "{} songs: {}",
            "Failing".bright_red().bold(),
            failed.display()
        );
        std::process::exit(1);
    }
}

fn trace_write_fail(err: io::Error) {
    eprintln!("Failed to write to trace file: {}", err);
    std::process::exit(2);
}

#[derive(Debug)]
pub struct Diagnostic<K> {
    when: Timestamp,
    pc: Address,
    level: DiagnosticLevel,
    kind: K,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
}

impl FromStr for DiagnosticLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("error") {
            Ok(Self::Error)
        } else if s.eq_ignore_ascii_case("warning") {
            Ok(Self::Warning)
        } else if s.eq_ignore_ascii_case("note") {
            Ok(Self::Note)
        } else {
            Err("unknown diagnostic level")
        }
    }
}

#[derive(Debug)]
enum BeforeOrAfter {
    Before,
    After,
    None,
}

impl FromStr for BeforeOrAfter {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("before") {
            Ok(Self::Before)
        } else if s.eq_ignore_ascii_case("after") {
            Ok(Self::After)
        } else if s.eq_ignore_ascii_case("none") {
            Ok(Self::None)
        } else {
            Err("must be either \"before\" or \"after\"")
        }
    }
}

fn parse_watch_arg(arg: &str) -> Result<(u16, u8), String> {
    let (addr, value) = arg
        .split_once('=')
        .ok_or_else(|| "expected \"ADDR=VALUE\", e.g. \"CAFE=2A\"".to_string())?;
    Ok((
        u16::from_str_radix(addr.trim(), 16).map_err(|err| format!("invalid address: {}", err))?,
        u8::from_str_radix(value.trim(), 16).map_err(|err| format!("invalid value: {}", err))?,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp {
    /// Tick 0 is the "init" phase.
    tick: u64,
    cycle: u16,
}

impl Display for DiagnosticLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "{}", "Error".bright_red()),
            Self::Warning => write!(f, "{}", "Warning".bright_yellow()),
            Self::Note => write!(f, "{}", "Note".bright_blue()),
        }
    }
}

struct SongIDs((u8, u8));

impl Display for SongIDs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 .0 == self.0 .1 {
            write!(f, "{}", self.0 .0)
        } else {
            write!(f, "{} and {}", self.0 .0, self.0 .1)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Address(u8, u16);

impl LowerHex for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.1 {
            0x4000..=0x7FFF => write!(f, "{:02x}:{:04x}", self.0, self.1),
            _ => write!(f, "00:{:04x}", self.1),
        }
    }
}
