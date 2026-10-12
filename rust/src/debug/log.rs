// SPDX-License-Identifier: GPL-2.0-only

//! Bounded in-memory log of enabled debug lines, shared by Rust and GDScript.
//!
//! Only enabled channels reach this module, so a quiet game never takes its lock. Each push
//! holds one mutex for O(1) work plus the stdout echo, which keeps sequence numbers in stdout
//! order. The oldest line is dropped once [`CAPACITY`] lines are stored.

use super::channel::Channel;
use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// Maximum number of buffered lines; older lines are dropped first.
pub const CAPACITY: usize = 4096;

/// Which side of the GDExtension boundary produced a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Rust simulation or bridge code.
    Rust,
    /// GDScript, through the Godot debug bridge.
    Godot,
}

impl Source {
    /// Short label used in dumps and the console: `rs` or `gd`.
    pub const fn label(self) -> &'static str {
        match self {
            Source::Rust => "rs",
            Source::Godot => "gd",
        }
    }
}

/// One buffered debug line.
#[derive(Clone, Debug)]
pub struct LogLine {
    /// Monotonic sequence number, starting at 1; never reused after the ring drops a line.
    pub seq: u64,
    /// Milliseconds since the first logged line or debug init.
    pub time_ms: u64,
    /// Channel the line was logged on.
    pub channel: Channel,
    /// Producer of the line.
    pub source: Source,
    /// Message text without the `[DEBUG:<tag>]` prefix.
    pub message: String,
}

struct LogRing {
    next_seq: u64,
    lines: VecDeque<LogLine>,
}

impl LogRing {
    const fn new() -> Self {
        Self {
            next_seq: 1,
            lines: VecDeque::new(),
        }
    }

    fn push(&mut self, channel: Channel, source: Source, message: String, time_ms: u64) -> u64 {
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.lines.push_back(LogLine {
            seq,
            time_ms,
            channel,
            source,
            message,
        });
        seq
    }

    // Lines are stored in sequence order, so the first newer line is a binary search away.
    fn since(&self, after: u64) -> impl Iterator<Item = &LogLine> {
        let start = self.lines.partition_point(|line| line.seq <= after);
        self.lines.range(start..)
    }
}

static RING: Mutex<LogRing> = Mutex::new(LogRing::new());
static START: OnceLock<Instant> = OnceLock::new();

fn ring() -> MutexGuard<'static, LogRing> {
    // A panic while logging must not take the debug log down with it.
    RING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts the log clock. Called by [`crate::debug::init`]; later calls are no-ops.
pub(crate) fn start_clock() {
    START.get_or_init(Instant::now);
}

fn elapsed_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Echoes and buffers one line. Callers check [`Channel::enabled`] first; this is the cold
/// path behind `debug_log!` and `traffic_log!`.
#[cold]
#[inline(never)]
pub fn emit(channel: Channel, args: fmt::Arguments<'_>) {
    push(channel, Source::Rust, args.to_string());
}

/// Echoes and buffers one already-formatted line if its channel is on. Returns `true` if
/// the line was kept.
pub fn push(channel: Channel, source: Source, message: String) -> bool {
    if !channel.enabled() {
        return false;
    }
    let time_ms = elapsed_ms();
    let mut ring = ring();
    if channel.prints_to_stdout() {
        // Byte-identical to the former `println!("[DEBUG:{}] {}", category, message)`.
        let _ = writeln!(
            std::io::stdout().lock(),
            "[DEBUG:{}] {}",
            channel.tag(),
            message
        );
    }
    ring.push(channel, source, message, time_ms);
    true
}

/// Calls `visit` for up to `max` lines newer than `after`, oldest first. Returns the newest
/// sequence number stored, or `after` when the ring is empty. O(log CAPACITY + max).
pub fn read_since(after: u64, max: usize, mut visit: impl FnMut(&LogLine)) -> u64 {
    let ring = ring();
    ring.since(after).take(max).for_each(&mut visit);
    ring.lines.back().map_or(after, |line| line.seq)
}

/// Drops every buffered line. Sequence numbers keep counting up.
pub fn clear() {
    ring().lines.clear();
}

/// Formats one line as dumps and the console's Copy action show it.
pub fn format_line(out: &mut String, line: &LogLine) {
    let _ = writeln!(
        out,
        "{:>6} {:>7}.{:03}s {} [DEBUG:{}] {}",
        line.seq,
        line.time_ms / 1000,
        line.time_ms % 1000,
        line.source.label(),
        line.channel.tag(),
        line.message
    );
}

/// Writes the channel state, visual and startup modes and every buffered line as a plain-text
/// report.
pub fn write_dump(path: &std::path::Path) -> std::io::Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# Metrum Rise debug dump");
    let _ = writeln!(out, "uptime_ms={}", elapsed_ms());
    out.push_str("\n## Channels\n");
    for &channel in Channel::ALL {
        let state = if channel.enabled() { "on" } else { "off" };
        let _ = writeln!(out, "{:<18} {state}", channel.tag());
    }
    out.push_str("\n## Visual modes\n");
    for mode in super::visual::MODES {
        let _ = writeln!(out, "{}={}", mode.name, mode.value_name(mode.index()));
    }
    out.push_str("\n## Startup modes\n");
    for (name, on) in super::startup_modes() {
        let _ = writeln!(out, "{name}={}", if *on { "on" } else { "off" });
    }
    out.push_str("\n## Log\n");
    {
        let ring = ring();
        for line in &ring.lines {
            format_line(&mut out, line);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_drops_oldest_and_reads_in_order() {
        let mut ring = LogRing::new();
        for i in 0..CAPACITY + 3 {
            ring.push(Channel::Road, Source::Rust, i.to_string(), 0);
        }
        assert_eq!(ring.lines.len(), CAPACITY);
        assert_eq!(ring.lines.front().unwrap().seq, 4);
        let tail: Vec<_> = ring.since(CAPACITY as u64).map(|line| line.seq).collect();
        assert_eq!(
            tail,
            [
                CAPACITY as u64 + 1,
                CAPACITY as u64 + 2,
                CAPACITY as u64 + 3
            ]
        );
        // A reader that fell behind the ring resumes at the oldest kept line.
        assert_eq!(ring.since(0).next().unwrap().seq, 4);
    }

    #[test]
    fn format_line_keeps_the_stdout_prefix() {
        let mut out = String::new();
        let line = LogLine {
            seq: 7,
            time_ms: 12_345,
            channel: Channel::Traffic,
            source: Source::Godot,
            message: "[JUNCTION_WAIT] a=1".into(),
        };
        format_line(&mut out, &line);
        assert_eq!(
            out,
            "     7      12.345s gd [DEBUG:traffic] [JUNCTION_WAIT] a=1\n"
        );
    }
}
