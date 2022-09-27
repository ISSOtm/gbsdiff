/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    io::{self, BufRead, BufReader},
    process::{Command, ExitStatus, Stdio},
};

use thiserror::Error;

use crate::{RegLog, RegWrite, RegWriteParseErr};

pub fn run_gbs(gbsplay_path: &str, gbs: &str) -> Result<Vec<RegLog>, GbsRunError> {
    let mut child = Command::new(gbsplay_path)
        .arg("-o")
        .arg("iodumper")
        .arg(gbs)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(GbsRunError::StartupErr)?;

    let output = BufReader::new(child.stdout.take().unwrap());
    let mut lines = output
        .lines()
        .enumerate()
        .map(|(line_no, res)| res.map(|line| (line_no + 1, line)));

    let mut logs = Vec::new();
    // Between tracks, the cycle counter is reset to 0, which causes the delta to be printed as a very large positive integer as well.
    // This is bad, but serves as a way to delineate tracks.
    // gbsplay reportedly allows only playing a subset of the tracks, but this appears not to work.
    loop {
        // Ignore all initialisation writes.
        for res in lines.by_ref() {
            let line = res?.1;
            if line.starts_with("subsong ") {
                break;
            }
        }

        let mut writes = Vec::new();
        let last = loop {
            if let Some(res) = lines.next() {
                let (line_no, line) = res?;
                if line.starts_with("ffffffff") || line.trim().is_empty() {
                    break false;
                }

                if let Some(write) = RegWrite::try_parse(&line)
                    .map_err(|err| GbsRunError::ParseError(line_no, line, err))?
                {
                    writes.push(write);
                }
            } else {
                break true;
            }
        };
        logs.push(RegLog::new(writes));

        if last {
            break;
        }
    }

    let status = child.wait().map_err(GbsRunError::WaitError)?;
    if !status.success() {
        return Err(GbsRunError::ExitFailure(status));
    }

    Ok(logs)
}

#[derive(Debug, Error)]
pub enum GbsRunError {
    #[error("Failed to start")]
    StartupErr(#[source] io::Error),
    #[error("gbsplay exited with code {0}")]
    ExitFailure(ExitStatus),
    #[error("Failed to read gbsplay's log: {0}")]
    ReadError(#[from] io::Error),
    #[error("Error parsing line {0} (\"{1}\"): {2}")]
    ParseError(usize, String, #[source] RegWriteParseErr),
    #[error("Error waiting for gbsplay: {0}")]
    WaitError(#[source] io::Error),
}
