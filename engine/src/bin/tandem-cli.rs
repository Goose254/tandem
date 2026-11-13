#![forbid(unsafe_code)]

use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use tandem_engine::engine::{Engine, OutputConfig, OutputHandle, SourceConfig};
use tandem_engine::{calibrate, devices, sessions};

/// Play your system audio on a second output, delayed and drift-corrected so both stay in sync.
#[derive(Parser)]
#[command(name = "tandem", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List playback devices.
    List,
    /// Mirror the source device's audio onto the target device, delayed.
    Run {
        /// Device whose audio is mirrored. Default: the Windows default output.
        /// This should be the slowest device (e.g. the Bluetooth amp).
        #[arg(short, long)]
        source: Option<String>,
        /// Device that receives the delayed copy (name or part of it, e.g. "Main Output").
        #[arg(short, long)]
        target: String,
        /// Delay applied to the target, in milliseconds.
        #[arg(short, long, default_value_t = 200)]
        delay: u32,
        /// Gain applied to the target, in dB.
        #[arg(short, long, default_value_t = 0.0, allow_negative_numbers = true)]
        gain_db: f32,
        /// Start with calibration clicks on (one blip per second on the source device).
        #[arg(short, long)]
        clicks: bool,
        /// Stop after this many seconds and print a status line every second (no keyboard UI).
        #[arg(long)]
        duration: Option<u64>,
    },
    /// Check the path "Line up with" uses: play two test sweeps on the targets only, cut off
    /// from the source, and report the timing. Only for devices nobody hears.
    Probe {
        /// Device to capture. It must stay silent: the sweeps never go through it.
        #[arg(short, long)]
        source: String,
        /// Devices that play the sweeps (repeat the option for several).
        #[arg(short, long, required = true)]
        target: Vec<String>,
        /// Seconds between the two sweeps.
        #[arg(long, default_value_t = 1.6)]
        spacing: f64,
    },
    /// List the programs playing on a device, as the Windows volume mixer sees them.
    /// Changes nothing.
    Sessions {
        /// Device name or part of it.
        device: String,
    },
    /// Mute the other programs playing on a device, wait, and unmute them, as a measurement
    /// started by hand does. Only for devices nobody hears: this writes to the Windows mixer
    /// and, unlike the app, keeps no record to recover from if it is killed halfway.
    QuietProbe {
        /// Device name or part of it.
        device: String,
        /// Seconds to keep them muted.
        #[arg(long, default_value_t = 2)]
        seconds: u64,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let _ = wasapi::initialize_mta();
    match cli.cmd {
        Cmd::List => list(),
        Cmd::Probe {
            source,
            target,
            spacing,
        } => probe(&source, &target, spacing),
        Cmd::Sessions { device } => {
            let dev = devices::find(Some(&device))?;
            println!("{}", dev.name);
            print_sessions(&dev.id)
        }
        Cmd::QuietProbe { device, seconds } => quiet_probe(&device, seconds),
        Cmd::Run {
            source,
            target,
            delay,
            gain_db,
            clicks,
            duration,
        } => {
            let src = devices::find(source.as_deref())?;
            let dst = devices::find(Some(&target))?;
            if src.id == dst.id {
                bail!("source and target are the same device ({})", src.name);
            }
            let source_fmt = devices::mix_format(&src.id)?;
            let target_fmt = devices::mix_format(&dst.id)?;
            println!("source: {} ({source_fmt})", src.name);
            println!("target: {} ({target_fmt})", dst.name);

            let mut engine = Engine::start(SourceConfig {
                device_id: src.id.clone(),
                fmt: source_fmt,
                follow_volume: true,
            })?;
            engine.set_clicks(clicks);
            engine.add_output(OutputConfig {
                key: dst.id.clone(),
                device_id: dst.id.clone(),
                fmt: target_fmt,
                delay_ms: delay,
                gain_db,
                muted: false,
            })?;
            let interactive = duration.is_none() && std::io::stdin().is_terminal();
            let result = if interactive {
                keyboard_ui(&engine, &dst.id)
            } else {
                timed_run(&engine, &dst.id, duration.unwrap_or(u64::MAX))
            };
            let out = engine.output(&dst.id).expect("output");
            let final_delay = out.delay_ms();
            let error = out.error().or_else(|| engine.source_error());
            engine.stop();
            result?;
            if let Some(e) = error {
                bail!("{e}");
            }
            println!(
                "\nfinal delay: {final_delay} ms. Next time:\n  tandem run --target \"{}\" --delay {final_delay}",
                dst.name
            );
            Ok(())
        }
    }
}

fn list() -> Result<()> {
    for d in devices::render_devices()? {
        let fmt = devices::mix_format(&d.id)
            .map(|f| f.to_string())
            .unwrap_or_else(|e| format!("format unavailable: {e}"));
        let mark = if d.is_default { "*" } else { " " };
        println!("{mark} {:<55} {fmt}", d.name);
    }
    println!("\n* = Windows default output");
    Ok(())
}

fn print_sessions(device_id: &str) -> Result<()> {
    let all = sessions::list(device_id)?;
    if all.is_empty() {
        println!("  (no session)");
    }
    for s in all {
        println!(
            "  pid {:>6} | {:<8} | {:<7} | {}{}{}",
            s.reference.process_id,
            if s.active { "playing" } else { "idle" },
            if s.muted { "muted" } else { "audible" },
            if s.own { "this program " } else { "" },
            if s.system_sounds {
                "system sounds "
            } else {
                ""
            },
            if sessions::should_mute(&s) {
                "<- a measurement would mute it"
            } else {
                ""
            },
        );
    }
    Ok(())
}

fn quiet_probe(device: &str, seconds: u64) -> Result<()> {
    let dev = devices::find(Some(device))?;
    println!("{}\nbefore:", dev.name);
    print_sessions(&dev.id)?;
    let candidates = sessions::candidates(&dev.id)?;
    let muted = sessions::mute(&candidates);
    println!("muted {muted} of {} candidate(s):", candidates.len());
    print_sessions(&dev.id)?;
    std::thread::sleep(Duration::from_secs(seconds));
    let unreached = sessions::unmute(&candidates);
    println!("restored, {} out of reach:", unreached.len());
    print_sessions(&dev.id)?;
    if !unreached.is_empty() {
        bail!("{} session(s) could not be unmuted", unreached.len());
    }
    Ok(())
}

fn probe(source: &str, targets: &[String], spacing_s: f64) -> Result<()> {
    const SWEEP_S: f64 = 1.0;
    let src = devices::find(Some(source))?;
    let source_fmt = devices::mix_format(&src.id)?;
    println!("source: {} ({source_fmt})", src.name);
    let mut engine = Engine::start(SourceConfig {
        device_id: src.id.clone(),
        fmt: source_fmt,
        follow_volume: true,
    })?;
    let mut keys = Vec::new();
    for target in targets {
        let dst = devices::find(Some(target))?;
        if dst.id == src.id {
            bail!("source and target are the same device ({})", src.name);
        }
        let fmt = devices::mix_format(&dst.id)?;
        println!("target: {} ({fmt})", dst.name);
        engine.add_output(OutputConfig {
            key: dst.id.clone(),
            device_id: dst.id.clone(),
            fmt,
            delay_ms: 50,
            gain_db: 0.0,
            muted: false,
        })?;
        keys.push(dst.id);
    }
    let ready = Instant::now();
    while !keys
        .iter()
        .all(|k| engine.output(k).is_some_and(|o| o.status().running))
    {
        if ready.elapsed() > Duration::from_secs(5) {
            bail!("the outputs did not start playing");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Let the drift controllers settle, as a measurement finds them.
    std::thread::sleep(Duration::from_secs(2));

    let glitches = |engine: &Engine| -> Vec<u64> {
        keys.iter()
            .map(|k| {
                let st = engine.output(k).expect("output").status();
                st.underruns + st.resyncs
            })
            .collect()
    };
    let sweep = Arc::new(calibrate::log_sweep(
        source_fmt.rate,
        SWEEP_S,
        120.0,
        9_000.0,
        0.35,
    ));
    engine.set_outputs_isolated(true);
    let discontinuities = engine.capture_discontinuities();
    let glitches_before = glitches(&engine);
    engine.take_source_peak();
    let mut loudest = vec![0f32; keys.len()];
    let mut listen = |engine: &Engine, seconds: f64| {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < until {
            for (level, key) in loudest.iter_mut().zip(&keys) {
                *level = level.max(engine.output(key).expect("output").status().level);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let first = Instant::now();
    engine.play_signal_to_outputs(Arc::clone(&sweep));
    listen(&engine, spacing_s);
    let between = first.elapsed().as_secs_f64();
    engine.play_signal_to_outputs(Arc::clone(&sweep));
    listen(&engine, SWEEP_S + 0.4);
    let starts = engine.output_signal_start_frames();
    let source_peak = engine.take_source_peak();
    engine.set_outputs_isolated(false);
    let lost = engine.capture_discontinuities() - discontinuities;
    let glitches_after = glitches(&engine);
    let error = engine.source_error();
    engine.stop();
    if let Some(e) = error {
        bail!("{e}");
    }
    if starts.len() != 2 {
        bail!("expected two sweeps to start, saw {}", starts.len());
    }
    let counted = (starts[1] - starts[0]) as f64 / source_fmt.rate as f64;
    println!(
        "\nsweeps started at output frames {} and {}",
        starts[0], starts[1]
    );
    println!(
        "spacing: {:.2} ms counted in frames, {:.2} ms by the wall clock (queued {:+.2} ms apart)",
        counted * 1000.0,
        between * 1000.0,
        (counted - between) * 1000.0
    );
    println!("capture discontinuities during the probe: {lost}");
    for ((key, level), (before, after)) in keys
        .iter()
        .zip(&loudest)
        .zip(glitches_before.iter().zip(&glitches_after))
    {
        let name = devices::render_devices()?
            .into_iter()
            .find(|d| &d.id == key)
            .map_or_else(|| key.clone(), |d| d.name);
        println!(
            "{name}: loudest level {level:.3}, dropouts and resyncs during the probe: {}",
            after - before
        );
    }
    println!("source: loudest captured level {source_peak:.5} (0 = it played nothing of ours)");
    Ok(())
}

fn alive(engine: &Engine, key: &str) -> bool {
    engine.source_alive() && engine.output(key).is_some_and(OutputHandle::is_alive)
}

fn status_line(engine: &Engine, key: &str) -> String {
    let out = engine.output(key).expect("output");
    let st = out.status();
    let state = if st.running { "playing" } else { "buffering" };
    format!(
        "delay {:>4} ms | {state:<9} | applied {:>6.1} ms | min lag {:>4.1} ms | clock corr {:>+5.0} ppm | underruns {} | resyncs {} | clicks {}",
        out.delay_ms(),
        st.buffered_ms,
        st.min_lag_ms,
        st.correction_ppm,
        st.underruns,
        st.resyncs,
        if engine.clicks() { "on" } else { "off" },
    )
}

fn timed_run(engine: &Engine, key: &str, seconds: u64) -> Result<()> {
    let start = Instant::now();
    while alive(engine, key) && start.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_secs(1));
        println!(
            "[{:>4}s] {}",
            start.elapsed().as_secs(),
            status_line(engine, key)
        );
    }
    Ok(())
}

fn keyboard_ui(engine: &Engine, key: &str) -> Result<()> {
    let out = engine.output(key).expect("output");
    println!(
        "\nkeys: up/down = delay ±1 ms, PgUp/PgDn = ±10 ms, c = calibration clicks, q = quit\n"
    );
    terminal::enable_raw_mode()?;
    let result = (|| -> Result<()> {
        let mut term = std::io::stdout();
        while alive(engine, key) {
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                let d = out.delay_ms() as i64;
                match key.code {
                    KeyCode::Up => out.set_delay_ms((d + 1) as u32),
                    KeyCode::Down => out.set_delay_ms((d - 1).max(0) as u32),
                    KeyCode::PageUp => out.set_delay_ms((d + 10) as u32),
                    KeyCode::PageDown => out.set_delay_ms((d - 10).max(0) as u32),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('c') => engine.set_clicks(!engine.clicks()),
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                }
            }
            write!(term, "\r{:<130}", status_line(engine, key))?;
            term.flush()?;
        }
        Ok(())
    })();
    terminal::disable_raw_mode()?;
    result
}
