// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Resource budgets for the single binary: under 2 s from start to ready,
//! under 80 MB resident and under 1 % of one CPU when idle (R-LT-1), from
//! the brief's targets. The brief's 40 MB download is the binary after
//! `gzip -9`, as packages, image layers and compressed downloads carry it
//! (ADR 0067, D279). The compressed image keeps release 3's 45 MB limit,
//! since it also carries the base layer and the local search model. The
//! binary on disk is capped just above its size, so growth fails too. A
//! shutdown signal must also drain and exit cleanly.
//!
//! `cargo xtask budgets --binary <path> [--image-bytes <n>] [--settle <secs>]`
//! starts `<path> serve` on a free port with an empty data folder, times
//! the first successful `GET /readyz`, waits for it to settle, reads its
//! resident set from `/proc` and samples its CPU time over a quiet window.
//! It then signs in, opens ten live-editing rooms and warms search (see
//! [`crate::load`]), reads the resident set again against the 250 MB busy
//! budget, and sends SIGTERM with the rooms still open. Until the binary
//! has a `serve` command only the size budgets apply, and the check says so.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The budgets, in bytes and time. Megabytes are decimal, as downloads are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The binary as it sits on disk.
    pub binary_bytes: u64,
    /// The binary compressed with `gzip -9`: what the brief's "40 MB
    /// download" measures (ADR 0067, D279).
    pub download_bytes: u64,
    /// The compressed container image.
    pub image_bytes: u64,
    /// Resident memory once idle.
    pub idle_rss_bytes: u64,
    /// CPU used once idle, in thousandths of one core.
    pub idle_cpu_permille: u64,
    /// Resident memory with ten live-editing rooms open and search warm.
    pub busy_rss_bytes: u64,
    /// From process start to the first ready response.
    pub cold_start: Duration,
    /// From SIGTERM to a clean exit.
    pub drain: Duration,
}

/// The budgets. The download, image, time and memory limits are the
/// brief's targets. The on-disk binary has no target of its own, so its
/// limit sits about 10 % above the size after #491 and catches growth.
pub const LIMITS: Limits = Limits {
    binary_bytes: 90_000_000,
    download_bytes: 40_000_000,
    image_bytes: 45_000_000,
    idle_rss_bytes: 80_000_000,
    idle_cpu_permille: 10,
    busy_rss_bytes: 250_000_000,
    cold_start: Duration::from_secs(2),
    drain: Duration::from_secs(30),
};

/// What one run measured. `None` means not measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measurements {
    /// Size of the binary file.
    pub binary_bytes: Option<u64>,
    /// Size of the binary after `gzip -9`.
    pub download_bytes: Option<u64>,
    /// Size of the compressed image, when given.
    pub image_bytes: Option<u64>,
    /// Time to the first ready response.
    pub cold_start: Option<Duration>,
    /// Resident memory after settling.
    pub idle_rss_bytes: Option<u64>,
    /// CPU used over the quiet window, in thousandths of one core.
    pub idle_cpu_permille: Option<u64>,
    /// Resident memory under the busy workload (see [`crate::load`]).
    pub busy_rss_bytes: Option<u64>,
    /// Time from SIGTERM to exit, and whether the exit was clean.
    pub drain: Option<(Duration, bool)>,
}

struct Options {
    binary: PathBuf,
    image_bytes: Option<u64>,
    settle: Duration,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut binary = None;
    let mut image_bytes = None;
    let mut settle = Duration::from_secs(10);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--binary" => binary = Some(PathBuf::from(value)),
            "--image-bytes" => {
                image_bytes = Some(value.parse().map_err(|e| format!("--image-bytes: {e}"))?);
            }
            "--settle" => {
                settle = Duration::from_secs(value.parse().map_err(|e| format!("--settle: {e}"))?);
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Options {
        binary: binary.ok_or("--binary is required")?,
        image_bytes,
        settle,
    })
}

/// Runs the budget check and prints a report.
pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let mut measured = Measurements {
        binary_bytes: Some(
            std::fs::metadata(&options.binary)
                .map_err(|e| format!("cannot read {}: {e}", options.binary.display()))?
                .len(),
        ),
        download_bytes: Some(gzip_bytes(&options.binary)?),
        image_bytes: options.image_bytes,
        ..Measurements::default()
    };
    let notice = if has_serve(&options.binary) {
        measure_server(&options.binary, options.settle, &mut measured)?;
        None
    } else {
        Some(
            "The binary has no `serve` command yet: cold start, idle memory and drain are checked once it does.",
        )
    };
    let failures = evaluate(&measured, &LIMITS);
    #[expect(clippy::print_stdout, reason = "xtask reports to the terminal")]
    {
        print!("{}", report(&measured, &LIMITS));
        if let Some(notice) = notice {
            println!("{notice}");
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("Budgets exceeded:\n  {}", failures.join("\n  ")))
    }
}

/// Lists every measurement over its limit.
pub fn evaluate(measured: &Measurements, limits: &Limits) -> Vec<String> {
    let mut failures = Vec::new();
    let mut over = |what: &str, value: u64, limit: u64| {
        if value >= limit {
            failures.push(format!(
                "{what}: {} over the {} budget",
                mb(value),
                mb(limit)
            ));
        }
    };
    if let Some(bytes) = measured.binary_bytes {
        over("binary size", bytes, limits.binary_bytes);
    }
    if let Some(bytes) = measured.download_bytes {
        over("gzipped binary", bytes, limits.download_bytes);
    }
    if let Some(bytes) = measured.image_bytes {
        over("compressed image", bytes, limits.image_bytes);
    }
    if let Some(bytes) = measured.idle_rss_bytes {
        over("idle resident memory", bytes, limits.idle_rss_bytes);
    }
    if let Some(bytes) = measured.busy_rss_bytes {
        over("busy resident memory", bytes, limits.busy_rss_bytes);
    }
    if let Some(permille) = measured.idle_cpu_permille
        && permille >= limits.idle_cpu_permille
    {
        failures.push(format!(
            "idle CPU: {} over the {} budget",
            percent(permille),
            percent(limits.idle_cpu_permille)
        ));
    }
    if let Some(time) = measured.cold_start
        && time >= limits.cold_start
    {
        failures.push(format!(
            "cold start: {} ms over the {} ms budget",
            time.as_millis(),
            limits.cold_start.as_millis()
        ));
    }
    if let Some((time, clean)) = measured.drain {
        if !clean {
            failures.push("shutdown: SIGTERM did not end in a clean exit".to_owned());
        } else if time >= limits.drain {
            failures.push(format!("shutdown: drain took {} ms", time.as_millis()));
        }
    }
    failures
}

fn report(measured: &Measurements, limits: &Limits) -> String {
    let row = |what: &str, value: Option<String>, limit: String| {
        format!(
            "{what:<22} {:>12}   budget {limit}\n",
            value.as_deref().unwrap_or("n/a")
        )
    };
    let mut text = String::new();
    text.push_str(&row(
        "binary size",
        measured.binary_bytes.map(mb),
        mb(limits.binary_bytes),
    ));
    text.push_str(&row(
        "gzipped binary",
        measured.download_bytes.map(mb),
        mb(limits.download_bytes),
    ));
    text.push_str(&row(
        "compressed image",
        measured.image_bytes.map(mb),
        mb(limits.image_bytes),
    ));
    text.push_str(&row(
        "cold start",
        measured.cold_start.map(|t| format!("{} ms", t.as_millis())),
        format!("{} ms", limits.cold_start.as_millis()),
    ));
    text.push_str(&row(
        "idle resident memory",
        measured.idle_rss_bytes.map(mb),
        mb(limits.idle_rss_bytes),
    ));
    text.push_str(&row(
        "idle CPU",
        measured.idle_cpu_permille.map(percent),
        percent(limits.idle_cpu_permille),
    ));
    text.push_str(&row(
        "busy resident memory",
        measured.busy_rss_bytes.map(mb),
        mb(limits.busy_rss_bytes),
    ));
    text.push_str(&row(
        "SIGTERM to exit",
        measured.drain.map(|(t, clean)| {
            format!(
                "{} ms{}",
                t.as_millis(),
                if clean { "" } else { " (unclean)" }
            )
        }),
        format!("{} ms", limits.drain.as_millis()),
    ));
    text
}

/// Formats bytes as decimal megabytes with one decimal place.
fn mb(bytes: u64) -> String {
    let tenths = bytes / 100_000;
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

/// Formats thousandths of a core as a percentage with one decimal place.
fn percent(permille: u64) -> String {
    format!("{}.{} %", permille / 10, permille % 10)
}

/// The size of `path` after `gzip -9`, the way release downloads, packages
/// and image layers carry it.
fn gzip_bytes(path: &Path) -> Result<u64, String> {
    let mut child = Command::new("gzip")
        .args(["-9", "-c", "--"])
        .arg(path)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run gzip: {e}"))?;
    let mut total = 0_u64;
    if let Some(mut out) = child.stdout.take() {
        let mut buffer = vec![0_u8; 1 << 16];
        loop {
            let read = out
                .read(&mut buffer)
                .map_err(|e| format!("cannot read gzip's output: {e}"))?;
            if read == 0 {
                break;
            }
            total += u64::try_from(read).unwrap_or(u64::MAX);
        }
    }
    let status = child.wait().map_err(|e| format!("gzip: {e}"))?;
    if status.success() {
        Ok(total)
    } else {
        Err(format!("gzip failed on {}: {status}", path.display()))
    }
}

fn has_serve(binary: &Path) -> bool {
    Command::new(binary)
        .args(["serve", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn measure_server(
    binary: &Path,
    settle: Duration,
    measured: &mut Measurements,
) -> Result<(), String> {
    let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|e| format!("no free port: {e}"))?
        .port();
    let data = std::env::temp_dir().join(format!("vellum-budgets-{}", std::process::id()));
    std::fs::create_dir_all(&data).map_err(|e| format!("cannot create {}: {e}", data.display()))?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let started = Instant::now();
    let mut child = Command::new(binary)
        .arg("serve")
        .env("VELLUM_BIND", address.to_string())
        .env("VELLUM_DATA_DIR", &data)
        .env("VELLUM_PROFILE", "single")
        .env("VELLUM_LOG", "warn")
        .env("VELLUM_KEK_PASSPHRASE", "budget-check-only")
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;
    let result = observe(&mut child, address, started, settle, measured);
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&data);
    result
}

fn observe(
    child: &mut Child,
    address: SocketAddr,
    started: Instant,
    settle: Duration,
    measured: &mut Measurements,
) -> Result<(), String> {
    // Allow five times the budget before giving up, so a slow start is
    // reported with its time instead of as a hang.
    let deadline = LIMITS.cold_start * 5;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("the server exited before it was ready: {status}"));
        }
        if get_status(address, "/readyz") == Some(200) {
            measured.cold_start = Some(started.elapsed());
            break;
        }
        if started.elapsed() > deadline {
            return Err(format!("not ready after {} ms", deadline.as_millis()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(settle);
    measured.idle_rss_bytes = Some(rss_bytes(child.id())?);
    measured.idle_cpu_permille = Some(idle_cpu(child.id())?);

    // The busy budget: ten rooms open and search warm. The rooms stay open
    // through the shutdown below, so the drain also covers live sockets.
    let rooms = crate::load::warm_up(address)?;
    std::thread::sleep(BUSY_SETTLE);
    measured.busy_rss_bytes = Some(rss_bytes(child.id())?);

    let signalled = Instant::now();
    let sent = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .map_err(|e| format!("cannot send SIGTERM: {e}"))?;
    if !sent.success() {
        return Err("kill -TERM failed".to_owned());
    }
    while signalled.elapsed() < LIMITS.drain {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            measured.drain = Some((signalled.elapsed(), status.success()));
            rooms.close();
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    measured.drain = Some((signalled.elapsed(), false));
    rooms.close();
    Ok(())
}

/// How long the idle CPU sample runs. Long enough that one clock tick
/// (10 ms) is 0.2 % of it, so the 1 % budget is five ticks.
const CPU_WINDOW: Duration = Duration::from_secs(5);

/// Clock ticks per second in `/proc/<pid>/stat`. Linux fixes `USER_HZ` at
/// 100 on every architecture it exposes to user space.
const TICKS_PER_SECOND: u64 = 100;

/// Samples the process's CPU time over [`CPU_WINDOW`], in thousandths of
/// one core.
fn idle_cpu(pid: u32) -> Result<u64, String> {
    let read = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|e| format!("cannot read the server's CPU time: {e}"))
            .and_then(|stat| cpu_ticks(&stat).ok_or_else(|| "unreadable /proc stat".to_owned()))
    };
    let before = read()?;
    let started = Instant::now();
    std::thread::sleep(CPU_WINDOW);
    let after = read()?;
    Ok(cpu_permille(
        after.saturating_sub(before),
        started.elapsed(),
    ))
}

/// User plus system clock ticks from `/proc/<pid>/stat` text. The command
/// name in brackets may hold spaces, so fields are counted after the last
/// closing bracket: `utime` and `stime` are the 12th and 13th there.
fn cpu_ticks(stat: &str) -> Option<u64> {
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    user.checked_add(system)
}

/// Converts clock ticks used over `window` to thousandths of one core.
fn cpu_permille(ticks: u64, window: Duration) -> u64 {
    let millis = u64::try_from(window.as_millis()).unwrap_or(u64::MAX).max(1);
    ticks.saturating_mul(1_000 * 1_000 / TICKS_PER_SECOND) / millis
}

/// Sends a minimal HTTP/1.1 GET and returns the status code.
/// How long the busy workload settles before its memory is read.
const BUSY_SETTLE: Duration = Duration::from_secs(2);

fn rss_bytes(pid: u32) -> Result<u64, String> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .map_err(|e| format!("cannot read the server's memory: {e}"))?;
    vm_rss_bytes(&status).ok_or_else(|| "no VmRSS line in /proc status".to_owned())
}

fn get_status(address: SocketAddr, path: &str) -> Option<u16> {
    let timeout = Duration::from_millis(500);
    let mut stream = TcpStream::connect_timeout(&address, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut head = [0_u8; 64];
    let read = stream.read(&mut head).ok()?;
    status_code(&String::from_utf8_lossy(head.get(..read)?))
}

/// Reads the status code from an HTTP status line.
pub fn status_code(response: &str) -> Option<u16> {
    let mut parts = response.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

/// Reads `VmRSS` (in kB) from `/proc/<pid>/status` text, as bytes.
fn vm_rss_bytes(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kilobytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_budget_passes() {
        let measured = Measurements {
            binary_bytes: Some(12_000_000),
            download_bytes: Some(4_000_000),
            image_bytes: Some(5_000_000),
            cold_start: Some(Duration::from_millis(300)),
            idle_rss_bytes: Some(30_000_000),
            idle_cpu_permille: Some(2),
            busy_rss_bytes: Some(120_000_000),
            drain: Some((Duration::from_millis(50), true)),
        };
        assert!(evaluate(&measured, &LIMITS).is_empty());
    }

    #[test]
    fn every_breach_is_listed() {
        let measured = Measurements {
            binary_bytes: Some(91_000_000),
            download_bytes: Some(41_000_000),
            image_bytes: Some(46_000_000),
            cold_start: Some(Duration::from_millis(2_500)),
            idle_rss_bytes: Some(90_000_000),
            idle_cpu_permille: Some(15),
            busy_rss_bytes: Some(260_000_000),
            drain: Some((Duration::from_millis(10), false)),
        };
        let failures = evaluate(&measured, &LIMITS);
        assert_eq!(failures.len(), 8, "{failures:?}");
        assert!(
            failures[0].contains("91.0 MB over the 90.0 MB"),
            "{failures:?}"
        );
        assert!(
            failures[1].contains("gzipped binary: 41.0 MB over the 40.0 MB"),
            "{failures:?}"
        );
        assert!(
            failures[4].contains("busy resident memory: 260.0 MB over the 250.0 MB"),
            "{failures:?}"
        );
        assert!(failures[5].contains("1.5 % over the 1.0 %"), "{failures:?}");
        assert!(failures[6].contains("2500 ms"), "{failures:?}");
    }

    #[test]
    fn unmeasured_values_are_skipped() {
        assert!(evaluate(&Measurements::default(), &LIMITS).is_empty());
        assert!(report(&Measurements::default(), &LIMITS).contains("n/a"));
    }

    #[test]
    fn gzip_measures_the_compressed_size() {
        let dir = std::env::temp_dir().join(format!("vellum-gzip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("zeros");
        std::fs::write(&file, vec![0_u8; 1_000_000]).unwrap();
        let bytes = gzip_bytes(&file).unwrap();
        assert!(bytes > 0 && bytes < 10_000, "{bytes}");
        assert!(gzip_bytes(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_vm_rss() {
        let status = "Name:\tvellum\nVmHWM:\t  50000 kB\nVmRSS:\t   41236 kB\nThreads:\t4\n";
        assert_eq!(vm_rss_bytes(status), Some(41_236 * 1024));
        assert_eq!(vm_rss_bytes("Name:\tvellum\n"), None);
    }

    #[test]
    fn reads_cpu_ticks() {
        // A command name with spaces and brackets must not shift the fields.
        let stat = "4242 (vellum (x) y) S 1 4242 4242 0 -1 4194560 900 0 0 0 37 12 0 0 20 0 9 0";
        assert_eq!(cpu_ticks(stat), Some(49));
        assert_eq!(cpu_ticks("4242 (vellum) S 1"), None);
        assert_eq!(cpu_ticks("no brackets"), None);
    }

    #[test]
    fn cpu_ticks_become_permille() {
        // Five ticks (50 ms) over five seconds is 1 % of a core.
        assert_eq!(cpu_permille(5, Duration::from_secs(5)), 10);
        assert_eq!(cpu_permille(0, Duration::from_secs(5)), 0);
        assert_eq!(cpu_permille(500, Duration::from_secs(5)), 1_000);
        assert_eq!(cpu_permille(1, Duration::ZERO), 10_000);
        assert_eq!(percent(10), "1.0 %");
    }

    #[test]
    fn reads_status_lines() {
        assert_eq!(status_code("HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(status_code("HTTP/1.1 503 Service Unavailable"), Some(503));
        assert_eq!(status_code("SSH-2.0-OpenSSH"), None);
        assert_eq!(status_code(""), None);
    }

    #[test]
    fn flags_are_checked() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--binary".to_owned()]).is_err());
        assert!(parse(&["--image-bytes".to_owned(), "x".to_owned()]).is_err());
        let options = parse(&[
            "--binary".to_owned(),
            "v".to_owned(),
            "--settle".to_owned(),
            "1".to_owned(),
        ])
        .unwrap();
        assert_eq!(options.settle, Duration::from_secs(1));
    }

    #[test]
    fn a_binary_without_serve_checks_size_only() {
        // `true` accepts any arguments, so use `false`: no serve command.
        assert!(!has_serve(Path::new("false")));
        assert!(run(&["--binary".to_owned(), "/nonexistent/vellum".to_owned()]).is_err());
    }

    #[test]
    fn nothing_listening_has_no_status() {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        assert_eq!(get_status(port, "/readyz"), None);
    }
}
