use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use tandem_engine::devices;
use tandem_engine::engine::{Engine, OutputConfig, OutputHandle, SourceConfig};

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
        Cmd::Run { source, target, delay, gain_db, clicks, duration } => {
            let src = devices::find(source.as_deref())?;
            let dst = devices::find(Some(&target))?;
            if src.id == dst.id {
                bail!("source and target are the same device ({})", src.name);
            }
            let source_fmt = devices::mix_format(&src.id)?;
            let target_fmt = devices::mix_format(&dst.id)?;
            println!("source: {} ({source_fmt})", src.name);
            println!("target: {} ({target_fmt})", dst.name);

            let mut engine = Engine::start(SourceConfig { device_id: src.id.clone(), fmt: source_fmt })?;
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

fn alive(engine: &Engine, key: &str) -> bool {
    engine.source_alive() && engine.output(key).is_some_and(OutputHandle::is_alive)
}

fn status_line(engine: &Engine, key: &str) -> String {
    let out = engine.output(key).expect("output");
    let st = out.status();
    let state = if st.running { "playing" } else { "buffering" };
    format!(
        "delay {:>4} ms | {state:<9} | buffered {:>6.1} ms | clock corr {:>+5.0} ppm | underruns {} | resyncs {} | clicks {}",
        out.delay_ms(),
        st.buffered_ms,
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
        println!("[{:>4}s] {}", start.elapsed().as_secs(), status_line(engine, key));
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
