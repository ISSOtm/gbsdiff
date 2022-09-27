/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use argh::FromArgs;
use owo_colors::OwoColorize;
use slicedisplay::SliceDisplay;

mod diff;
mod gbs;
mod reg_log;
use reg_log::*;

use crate::diff::DiagnosticLevel;

#[derive(FromArgs)]
/// Analyze differences in audio register writes between two GBS files.
struct Args {
    #[argh(option, default = "\"gbsplay\".into()")]
    /// how `gbsplay` will be invoked
    gbsplay_path: String,
    #[argh(switch, short = 'p')]
    /// whether to report even benign differences
    pedantic: bool,
    #[argh(option, short = 'm', default = "1000")]
    /// how many diagnostics to report per song, at most
    max_reports: usize,

    #[argh(positional)]
    /// path to the GBS file that was built before the changes
    before: String,
    #[argh(positional)]
    /// path to the GBS file that was built after the changes
    after: String,
}
fn main() {
    let args: Args = argh::from_env();

    let run = |path| {
        eprintln!(
            "{} {} {}...",
            "==>".bold(),
            "Collecting".bright_blue().bold(),
            &path
        );

        match gbs::run_gbs(&args.gbsplay_path, path) {
            Ok(log) => log,
            Err(e) => {
                eprintln!(
                    "{} while running gbsplay on {}: {}",
                    "error".bright_red().bold(),
                    path,
                    e
                );
                std::process::exit(2);
            }
        }
    };
    let before_logs = run(&args.before);
    let after_logs = run(&args.after);

    if before_logs.len() != after_logs.len() {
        eprintln!(
            "{}: \"Before\" log has {} entries, \"after\" has {}; hopefully none are mismatched",
            "warning".bright_yellow().bold(),
            before_logs.len(),
            after_logs.len()
        );
    }

    let mut failed = Vec::new();
    for (i, (before_log, after_log)) in before_logs.iter().zip(after_logs.iter()).enumerate() {
        eprintln!(
            "{} {} tracks {}...",
            "==>".bold(),
            "Comparing".bright_cyan().bold(),
            i.bold()
        );

        let diagnostics = diff::diff(before_log, after_log, args.max_reports, args.pedantic);
        let mut ok = true;
        let mut frame = u32::MAX;
        for diagnostic in &diagnostics {
            if diagnostic.level != DiagnosticLevel::Pedantic {
                ok = false;
            }

            // Print the diagnostic
            if diagnostic.frame != frame {
                frame = diagnostic.frame;
                eprintln!("{} Frame {} {}", "==".bold(), frame, "==".bold());
            }
            eprintln!(
                "{} on cycle {}: {}",
                diagnostic.level, diagnostic.cycles, diagnostic.kind
            );
        }
        if diagnostics.len() == args.max_reports {
            eprintln!(
                "...stopping at {} diagnostics. Go fix your code!",
                args.max_reports
            );
        }

        if ok {
            eprintln!("{}", "OK!".bright_green().bold());
        } else {
            failed.push(i);
        }
    }

    if failed.is_empty() {
        eprintln!(
            "{} {}",
            "==>".bold(),
            "All tracks OK!".bright_green().bold()
        );
    } else {
        eprintln!(
            "{} track(s): {}",
            "Failing".bright_red().bold(),
            failed.display()
        );
        std::process::exit(1);
    }
}
