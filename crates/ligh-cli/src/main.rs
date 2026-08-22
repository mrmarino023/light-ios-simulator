use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use ligh_core::{
    default_sock_path, ensure_daemon, sibling_lighd, AccessibilityTree, DaemonClient, DevicePreset,
    FeatureRequirements, FrameMeta, LighConfig, ObserveSnapshot, SessionState,
};
use ligh_gpu::{FrameCompositor, Screenshot};
use ligh_host::{AxDump, HidInput, HostSession};
use ligh_sim::{ensure_headless, Benchmark, SimSupervisor, Simctl};
use tracing_subscriber::EnvFilter;

mod agent_bench;
mod fair_bench;

// ────────────────────────── CLI definition ────────────────────────────────────

#[derive(Parser)]
#[command(name = "ligh", about = "LIGH v3 — GPU-native real iOS sim", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Output as JSON (for agent / CI use).
    #[arg(long, global = true)]
    json: bool,
    /// Bypass `lighd` and hit host APIs in-process (cold path — for benchmarks).
    #[arg(long, global = true)]
    direct: bool,
}

#[derive(Subcommand)]
enum Commands {
    Doctor,
    Device {
        #[command(subcommand)]
        command: DeviceCommands,
    },
    Up {
        #[arg(short, long, value_enum, default_value = "iphone-15-pro")]
        device: DeviceArg,
        #[arg(long)]
        udid: Option<String>,
        #[arg(long)]
        requires: Option<String>,
        #[arg(long)]
        gui: bool,
    },
    Status,
    Down,
    Run {
        app: String,
        #[arg(long)]
        bundle_id: Option<String>,
    },
    Relaunch,
    /// Interactive Metal window — see + touch your real iOS sim.
    Gui {
        #[arg(short, long, value_enum, default_value = "iphone-15-pro")]
        device: DeviceArg,
        #[arg(long)]
        udid: Option<String>,
        /// Open window ~5s, present frames, exit 0/1 (CI / proof).
        #[arg(long)]
        verify: bool,
    },
    /// Probe v3 GPU path: private boot + IOSurface → Metal (no Simulator.app).
    Probe {
        #[arg(short, long, value_enum, default_value = "iphone-15-pro")]
        device: DeviceArg,
    },
    /// Benchmarks. Prefer `ligh bench agent` — the reproducible agent-workflow proof.
    Bench {
        #[command(subcommand)]
        command: BenchCommands,
    },
    Sim {
        #[command(subcommand)]
        command: SimCommands,
    },

    // ── Agent-oriented primitives ─────────────────────────────────────────────

    /// Inject a tap. Coordinates default to normalized 0..1; use --points for pixel coords.
    Tap {
        #[arg(long)]
        x: Option<f64>,
        #[arg(long)]
        y: Option<f64>,
        /// Treat x/y as points instead of 0..1 normalized.
        #[arg(long)]
        points: bool,
        /// Tap center of first AX element matching this label (waits up to --timeout-ms).
        #[arg(long)]
        label: Option<String>,
        /// Tap by stable scene-graph id from observe v2.
        #[arg(long)]
        id: Option<String>,
        /// AX wait budget for --label/--id (default 2000).
        #[arg(long, default_value_t = 2000)]
        timeout_ms: u64,
    },
    /// Long-press (context menus). Prefer --id / --label.
    #[command(name = "long-press")]
    LongPress {
        #[arg(long)]
        x: Option<f64>,
        #[arg(long)]
        y: Option<f64>,
        #[arg(long)]
        points: bool,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 600)]
        hold_ms: u64,
        #[arg(long, default_value_t = 2000)]
        timeout_ms: u64,
    },
    /// Swipe until label/id is on-screen.
    #[command(name = "scroll-until")]
    ScrollUntil {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 8)]
        max_swipes: u32,
        #[arg(long, default_value_t = 12000)]
        timeout_ms: u64,
    },
    /// Dump accessibility tree (AXPTranslator, headless).
    Ax,
    /// Block until an AX label/identifier or id is visible.
    Wait {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 8000)]
        timeout_ms: u64,
    },
    /// Query whether an AX label/id currently exists (no wait).
    Exists {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Type UTF-8 text via IndigoHID keyboard.
    Type {
        #[arg(long)]
        text: String,
    },
    /// Clear focused field via repeated Delete.
    Clear {
        #[arg(long, default_value_t = 40)]
        count: u32,
    },
    /// Press a named key (return|delete|escape|tab|space|arrows).
    Key {
        #[arg(long)]
        name: String,
    },
    /// Recent sensation events only.
    Sense,
    /// Inject a swipe gesture (down → move → up).
    Swipe {
        #[arg(long)]
        from_x: f64,
        #[arg(long)]
        from_y: f64,
        #[arg(long)]
        to_x: f64,
        #[arg(long)]
        to_y: f64,
        #[arg(long)]
        points: bool,
    },
    /// Press the Home button.
    Home,
    /// Capture screenshot from IOSurface (debug — not agent happy path).
    Screenshot {
        /// Path to write PNG. Defaults to `~/.ligh/screenshot.png`.
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Structured observation snapshot (JSON-first). Prefer hot `lighd`.
    Observe {
        /// Skip accessibility dump (frame + session only).
        #[arg(long)]
        no_ax: bool,
        /// Wait until AX is settled (ready + actionable). Default 2500ms on hot path.
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
        /// Do not settle (single dump).
        #[arg(long)]
        no_settle: bool,
    },
    /// Control-plane: recover until Ready (home+settle) or structured fault.
    Ready {
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
        #[arg(long, default_value_t = 6)]
        recover_homes: u32,
    },
    /// Capability ops (act-with-settle contract).
    Cap {
        #[command(subcommand)]
        command: CapCommands,
    },
    /// Start / stop / status of the persistent agent host (`lighd`).
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
    /// Alias for `ligh bench agent` (kept for scripts).
    #[command(name = "agent-bench")]
    AgentBench {
        #[arg(long, default_value_t = 8)]
        iterations: u32,
        #[arg(long, default_value_t = true)]
        vs_cold: bool,
        #[arg(long)]
        no_cold: bool,
        #[arg(long)]
        micro_only: bool,
        #[arg(long)]
        workload_only: bool,
        #[arg(long, default_value_t = 40)]
        steps: u32,
        #[arg(long)]
        no_wda: bool,
    },
}

#[derive(Subcommand)]
enum CapCommands {
    /// Open Settings (IT/EN) and assert surface=settings.
    #[command(name = "open-settings")]
    OpenSettings {
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
    },
    /// Settings search → type query → settle (Bluetooth etc.).
    #[command(name = "settings-search")]
    SettingsSearch {
        query: String,
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
    },
    /// Assert scene.surface after settle.
    #[command(name = "assert-surface")]
    AssertSurface {
        surface: String,
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
    },
    /// Settle → tap label/id → settle.
    Tap {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
    },
    /// Settle → type → settle.
    Type {
        #[arg(long)]
        text: String,
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
    },
    /// Install Debug `.app` → launch → settle → optional wait label (product path).
    #[command(name = "run-app")]
    RunApp {
        app: String,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        wait_label: Option<String>,
        #[arg(long)]
        wait_id: Option<String>,
        #[arg(long, default_value_t = 3500)]
        settle_ms: u64,
        #[arg(long, default_value_t = 8000)]
        timeout_ms: u64,
        /// Skip simctl install (relaunch only).
        #[arg(long, default_value_t = false)]
        no_install: bool,
        /// Extra argv for simctl launch (repeatable).
        #[arg(long = "launch-arg")]
        launch_args: Vec<String>,
    },
    /// Settle → wait until AX label exists.
    #[command(name = "wait-label")]
    WaitLabel {
        label: String,
        #[arg(long, default_value_t = 2500)]
        settle_ms: u64,
        #[arg(long, default_value_t = 8000)]
        timeout_ms: u64,
    },
    /// Install+launch then motor steps JSON (product job — one capability).
    #[command(name = "app-job")]
    AppJob {
        app: String,
        #[arg(long)]
        bundle_id: Option<String>,
        /// JSON array of steps: wait/tap/type with id|label|text.
        #[arg(long)]
        steps: String,
        #[arg(long, default_value_t = 3500)]
        settle_ms: u64,
        #[arg(long, default_value_t = 10000)]
        timeout_ms: u64,
        /// Skip simctl install (relaunch only).
        #[arg(long, default_value_t = false)]
        no_install: bool,
        /// Extra argv for simctl launch (repeatable).
        #[arg(long = "launch-arg")]
        launch_args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum BenchCommands {
    /// Stock Simulator.app vs LIGH compositor (legacy GPU path bench).
    Boot {
        #[arg(short, long, value_enum, default_value = "iphone-15-pro")]
        device: DeviceArg,
    },
    /// Killer reproducible agent workload: LIGHd vs simctl/MCP (WDA slot when wired).
    Agent {
        /// Microbench iterations for per-op p50/p95 (observe/tap/exists/screenshot).
        #[arg(long, default_value_t = 8)]
        iterations: u32,
        /// Measure cold MCP-style path. On unless `--no-cold`.
        #[arg(long, default_value_t = true)]
        vs_cold: bool,
        /// Skip cold-path comparison.
        #[arg(long)]
        no_cold: bool,
        /// Skip the 30–50 step structured workload (microbench only).
        #[arg(long)]
        micro_only: bool,
        /// Skip microbench primitives; run workload + comparisons only (default on).
        #[arg(long, default_value_t = true)]
        workload_only: bool,
        /// Also run per-op microbench (observe/tap/screenshot p50).
        #[arg(long)]
        with_micro: bool,
        /// Target step count for the structured workflow (clamped 20..=60).
        #[arg(long, default_value_t = 40)]
        steps: u32,
        /// Skip WDA/Appium even if Appium is listening.
        #[arg(long)]
        no_wda: bool,
    },
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Ensure `lighd` is running (spawn if needed).
    Start,
    /// Ping `~/.ligh/lighd.sock`.
    Status,
    /// Ask daemon to shut down the sim session and exit.
    Stop,
}

#[derive(Subcommand)]
enum DeviceCommands {
    Create {
        #[arg(short, long, value_enum, default_value = "iphone-15-pro")]
        device: DeviceArg,
    },
}

#[derive(Subcommand)]
enum SimCommands {
    Measure,
}

#[derive(Clone, ValueEnum)]
enum DeviceArg {
    #[value(name = "iphone-se")]
    IphoneSe,
    #[value(name = "iphone-15-pro")]
    Iphone15Pro,
    #[value(name = "iphone-15-pro-max")]
    Iphone15ProMax,
    #[value(name = "ipad-pro-11")]
    IpadPro11,
}

impl From<DeviceArg> for DevicePreset {
    fn from(v: DeviceArg) -> Self {
        match v {
            DeviceArg::IphoneSe => DevicePreset::IphoneSe,
            DeviceArg::Iphone15Pro => DevicePreset::Iphone15Pro,
            DeviceArg::Iphone15ProMax => DevicePreset::Iphone15ProMax,
            DeviceArg::IpadPro11 => DevicePreset::IpadPro11,
        }
    }
}

// ────────────────────────── main ──────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    let config = LighConfig::load()?;
    let use_json = cli.json;
    let direct = cli.direct;

    match cli.command {
        Commands::Doctor => {
            SimSupervisor::new(config).doctor()?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "hint": "ligh device create && ligh up" }));
            } else {
                println!("✓ run: ligh device create && ligh up");
            }
        }

        Commands::Device { command } => match command {
            DeviceCommands::Create { device } => {
                let s = SimSupervisor::new(config).device_create(device.into())?;
                if use_json {
                    println!("{}", serde_json::json!({ "name": s.name, "udid": s.udid }));
                } else {
                    println!("✓ {} ({})", s.name, s.udid);
                }
            }
        },

        Commands::Up { device, udid, requires, gui } => {
            let req = requires
                .as_deref()
                .map(FeatureRequirements::parse_csv)
                .unwrap_or_default();
            let sup = SimSupervisor::new(config).with_requirements(req);
            if !gui {
                ensure_headless();
            }
            let t = std::time::Instant::now();
            let s = sup.up(device.into(), !gui, udid.as_deref())?;
            let fp = sup.measure().ok();
            if use_json {
                println!("{}", serde_json::json!({
                    "udid": s.udid,
                    "boot_secs": t.elapsed().as_secs_f64(),
                    "ram_mb": fp.as_ref().map(|f| f.total_mb),
                    "slim": s.slim_applied,
                }));
            } else {
                println!("✓ headless ready in {:.1}s", t.elapsed().as_secs_f64());
                println!("  udid: {}", s.udid);
                if let Some(fp) = fp {
                    println!("  RAM:  {:.0} MB ({} procs)", fp.total_mb, fp.process_count);
                }
                if s.slim_applied {
                    println!("  slim: disabledJob + runtime (session)");
                }
            }
        }

        Commands::Status => {
            let st = SimSupervisor::new(config).status()?;
            if use_json {
                println!("{}", serde_json::to_string_pretty(&st)?);
            } else {
                print_status_report(&st);
            }
        }

        Commands::Down => {
            SimSupervisor::new(config).down()?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true }));
            } else {
                println!("✓ down");
            }
        }

        Commands::Run { app, bundle_id } => {
            let s = SimSupervisor::new(config).install_and_launch(app.as_ref(), bundle_id.as_deref())?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "bundle_id": s.app_bundle_id }));
            } else {
                println!("✓ launched");
            }
        }

        Commands::Relaunch => {
            let d = SimSupervisor::new(config).relaunch()?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "ms": d.as_millis() }));
            } else {
                println!("✓ relaunch {:.0}ms", d.as_millis());
            }
        }

        Commands::Gui { device, udid, verify } => {
            if !use_json {
                if verify {
                    println!("LIGH GUI verify — boot + Metal window smoke test…");
                } else {
                    println!("LIGH GUI — IOSurface → Metal + touch…");
                }
            }
            ligh_runtime::GpuSession::run_gui(device.into(), udid.as_deref(), verify)?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true }));
            } else if verify {
                println!("✓ GUI verify ok — Metal window presented sim frames");
            }
        }

        Commands::Probe { device } => {
            if !use_json {
                println!("probe: IOSurface → Metal headless path…");
            }
            let r = ligh_runtime::GpuSession::probe(device.into())?;
            let sim_app = std::process::Command::new("pgrep")
                .args(["-x", "Simulator"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if use_json {
                println!("{}", serde_json::json!({
                    "ok": r.compositor.imports_fail == 0 && r.compositor.imports_ok >= 30,
                    "udid": r.udid,
                    "boot_secs": r.boot_secs,
                    "stream": { "frames": r.stream.frames, "fps": r.stream.fps, "width": r.stream.width, "height": r.stream.height },
                    "metal": { "imports_ok": r.compositor.imports_ok, "imports_fail": r.compositor.imports_fail, "fps": r.compositor.fps },
                    "hid_tap_ok": r.hid_tap_ok,
                    "simulator_app_running": sim_app,
                }));
            } else {
                println!();
                println!("✓ GPU session live");
                println!("  udid:       {}", r.udid);
                println!("  boot:       {:.1}s (private CoreSimulator, no Simulator.app)", r.boot_secs);
                println!("  stream:     {} frames @ {:.1} fps ({}×{}) over {:.1}s",
                    r.stream.frames, r.stream.fps, r.stream.width, r.stream.height, r.sample_secs);
                println!("  metal:      {} imports ok, {} fail @ {:.1} fps",
                    r.compositor.imports_ok, r.compositor.imports_fail, r.compositor.fps);
                println!("  hid tap:    {}", if r.hid_tap_ok { "ok (IndigoHID center tap)" } else { "FAILED" });
                println!("  Simulator.app running: {}", if sim_app { "YES (unexpected)" } else { "no ✓" });
            }
            if r.compositor.imports_fail > 0 || r.compositor.imports_ok < 30 {
                std::process::exit(1);
            }
        }

        Commands::Bench { command } => match command {
            BenchCommands::Boot { device } => {
                if !use_json {
                    println!("bench boot: stock (Simulator.app) vs LIGH (no Simulator.app)…");
                }
                let r = Benchmark::run(device.into())?;
                if use_json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    print_bench(&r);
                }
            }
            BenchCommands::Agent {
                iterations,
                vs_cold,
                no_cold,
                micro_only,
                workload_only,
                with_micro,
                steps,
                no_wda,
            } => {
                let client = hot_client()?;
                agent_bench::run_agent_bench(
                    &config,
                    client,
                    agent_bench::AgentBenchOpts {
                        iterations,
                        vs_cold: vs_cold && !no_cold,
                        micro_only,
                        workload_only: workload_only && !with_micro,
                        steps,
                        use_json,
                        no_wda,
                    },
                )?;
            }
        },

        Commands::Sim { command } => match command {
            SimCommands::Measure => {
                let r = SimSupervisor::new(config).measure()?;
                if use_json {
                    println!("{}", serde_json::json!({ "total_mb": r.total_mb, "process_count": r.process_count }));
                } else {
                    println!("{:.0} MB, {} procs", r.total_mb, r.process_count);
                }
            }
        },

        // ── Agent-oriented commands (prefer hot lighd) ────────────────────────

        Commands::Tap {
            x,
            y,
            points,
            label,
            id,
            timeout_ms,
        } => {
            if let Some(eid) = id {
                if direct {
                    let session = require_session(&config)?;
                    let (sim_w, sim_h) = session_dims(&session);
                    let (nx, ny, waited) = AxDump::wait_id(
                        &session.udid,
                        &eid,
                        Duration::from_millis(timeout_ms),
                    )?;
                    HidInput::tap(&session.udid, nx, ny, sim_w, sim_h)?;
                    if use_json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": true, "id": eid, "x": nx, "y": ny,
                                "waited_ms": waited.as_secs_f64() * 1000.0, "path": "direct"
                            })
                        );
                    } else {
                        println!(
                            "✓ tap id={eid:?} → ({nx:.3}, {ny:.3}) waited {:.0}ms",
                            waited.as_secs_f64() * 1000.0
                        );
                    }
                } else {
                    let data = hot_client()?.tap_id(&eid, Some(timeout_ms))?;
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true, "data": data, "path": "lighd" }));
                    } else {
                        println!("✓ tap id={eid:?} via lighd {data}");
                    }
                }
            } else if let Some(label) = label {
                if direct {
                    let session = require_session(&config)?;
                    let (sim_w, sim_h) = session_dims(&session);
                    let (nx, ny, waited) = AxDump::wait_label(
                        &session.udid,
                        &label,
                        Duration::from_millis(timeout_ms),
                    )?;
                    HidInput::tap(&session.udid, nx, ny, sim_w, sim_h)?;
                    if use_json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": true, "label": label, "x": nx, "y": ny,
                                "waited_ms": waited.as_secs_f64() * 1000.0, "path": "direct"
                            })
                        );
                    } else {
                        println!(
                            "✓ tap label={label:?} → ({nx:.3}, {ny:.3}) waited {:.0}ms",
                            waited.as_secs_f64() * 1000.0
                        );
                    }
                } else {
                    let data = hot_client()?.tap_label(&label, Some(timeout_ms))?;
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true, "data": data, "path": "lighd" }));
                    } else {
                        println!("✓ tap label={label:?} via lighd {data}");
                    }
                }
            } else {
                let x = x.ok_or_else(|| anyhow::anyhow!("--x required (or use --label/--id)"))?;
                let y = y.ok_or_else(|| anyhow::anyhow!("--y required (or use --label/--id)"))?;
                let normalized = !points;
                if direct {
                    let session = require_session(&config)?;
                    let (sim_w, sim_h) = session_dims(&session);
                    let (nx, ny) = if points { (x / sim_w, y / sim_h) } else { (x, y) };
                    HidInput::tap(&session.udid, nx, ny, sim_w, sim_h)?;
                } else {
                    hot_client()?.tap(x, y, normalized)?;
                }
                if use_json {
                    println!("{}", serde_json::json!({ "ok": true, "x": x, "y": y, "path": if direct { "direct" } else { "lighd" } }));
                } else {
                    println!("✓ tap ({x:.3}, {y:.3}) via {}", if direct { "direct" } else { "lighd" });
                }
            }
        }

        Commands::LongPress {
            x,
            y,
            points,
            label,
            id,
            hold_ms,
            timeout_ms,
        } => {
            if direct {
                let session = require_session(&config)?;
                let (sim_w, sim_h) = session_dims(&session);
                let (nx, ny) = if let Some(ref eid) = id {
                    let (nx, ny, _) =
                        AxDump::wait_id(&session.udid, eid, Duration::from_millis(timeout_ms))?;
                    (nx, ny)
                } else if let Some(ref lab) = label {
                    let (nx, ny, _) =
                        AxDump::wait_label(&session.udid, lab, Duration::from_millis(timeout_ms))?;
                    (nx, ny)
                } else {
                    let x = x.ok_or_else(|| anyhow::anyhow!("--x or --label/--id required"))?;
                    let y = y.ok_or_else(|| anyhow::anyhow!("--y or --label/--id required"))?;
                    if points {
                        (x / sim_w, y / sim_h)
                    } else {
                        (x, y)
                    }
                };
                HidInput::tap_hold(&session.udid, nx, ny, sim_w, sim_h, hold_ms as f64)?;
                if use_json {
                    println!("{}", serde_json::json!({ "ok": true, "x": nx, "y": ny, "hold_ms": hold_ms }));
                } else {
                    println!("✓ long-press ({nx:.3}, {ny:.3}) hold={hold_ms}ms");
                }
            } else {
                let data = hot_client()?.call(&ligh_core::DaemonRequest::LongPress {
                    x: x.unwrap_or(0.0),
                    y: y.unwrap_or(0.0),
                    normalized: !points,
                    label,
                    id,
                    hold_ms: Some(hold_ms),
                    timeout_ms: Some(timeout_ms),
                })?;
                let v = data.into_result()?;
                if use_json {
                    println!("{}", serde_json::json!({ "ok": true, "data": v }));
                } else {
                    println!("✓ long-press via lighd {v:?}");
                }
            }
        }

        Commands::ScrollUntil {
            label,
            id,
            max_swipes,
            timeout_ms,
        } => {
            if label.is_none() && id.is_none() {
                anyhow::bail!("scroll-until needs --label or --id");
            }
            let data = hot_client()?.scroll_until(
                label.as_deref(),
                id.as_deref(),
                Some(max_swipes),
                Some(timeout_ms),
            )?;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "data": data }));
            } else {
                println!("✓ scroll-until {data}");
            }
        }

        Commands::Wait {
            label,
            id,
            timeout_ms,
        } => {
            if let Some(eid) = id {
                if direct {
                    let session = require_session(&config)?;
                    let (nx, ny, waited) =
                        AxDump::wait_id(&session.udid, &eid, Duration::from_millis(timeout_ms))?;
                    if use_json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": true, "id": eid, "x": nx, "y": ny,
                                "waited_ms": waited.as_secs_f64() * 1000.0, "path": "direct"
                            })
                        );
                    } else {
                        println!(
                            "✓ wait id={eid:?} ({nx:.3}, {ny:.3}) in {:.0}ms",
                            waited.as_secs_f64() * 1000.0
                        );
                    }
                } else {
                    let data = hot_client()?.wait_id(&eid, Some(timeout_ms))?;
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true, "data": data }));
                    } else {
                        println!("✓ wait id={eid:?} {data}");
                    }
                }
            } else if let Some(label) = label {
                if direct {
                    let session = require_session(&config)?;
                    let (nx, ny, waited) =
                        AxDump::wait_label(&session.udid, &label, Duration::from_millis(timeout_ms))?;
                    if use_json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": true, "label": label, "x": nx, "y": ny,
                                "waited_ms": waited.as_secs_f64() * 1000.0, "path": "direct"
                            })
                        );
                    } else {
                        println!(
                            "✓ wait {label:?} ({nx:.3}, {ny:.3}) in {:.0}ms",
                            waited.as_secs_f64() * 1000.0
                        );
                    }
                } else {
                    let data = hot_client()?.wait_label(&label, Some(timeout_ms))?;
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true, "data": data }));
                    } else {
                        println!("✓ wait {label:?} {data}");
                    }
                }
            } else {
                anyhow::bail!("wait needs --label or --id");
            }
        }

        Commands::Exists { label, id } => {
            let found = if let Some(ref eid) = id {
                if direct {
                    let session = require_session(&config)?;
                    AxDump::exists_id(&session.udid, eid)?
                } else {
                    hot_client()?
                        .call(&ligh_core::DaemonRequest::Exists {
                            label: None,
                            id: Some(eid.clone()),
                        })?
                        .into_result()?
                        .and_then(|d| d.get("found").and_then(|v| v.as_bool()))
                        .unwrap_or(false)
                }
            } else if let Some(ref label) = label {
                if direct {
                    let session = require_session(&config)?;
                    AxDump::exists_label(&session.udid, label)?
                } else {
                    hot_client()?.exists_label(label)?
                }
            } else {
                anyhow::bail!("exists needs --label or --id");
            };
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "found": found, "label": label, "id": id }));
            } else {
                println!("{}", if found { "found" } else { "missing" });
            }
        }

        Commands::Clear { count } => {
            if direct {
                let session = require_session(&config)?;
                HidInput::clear(&session.udid, count)?;
            } else {
                hot_client()?.clear(Some(count))?;
            }
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "count": count }));
            } else {
                println!("✓ clear {count} deletes");
            }
        }

        Commands::Key { name } => {
            if direct {
                let session = require_session(&config)?;
                HidInput::key_named(&session.udid, &name)?;
            } else {
                hot_client()?.key(&name)?;
            }
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "key": name }));
            } else {
                println!("✓ key {name}");
            }
        }

        Commands::Sense => {
            let data = hot_client()?.sense()?;
            if use_json {
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                println!("{data}");
            }
        }

        Commands::Type { text } => {
            if direct {
                let session = require_session(&config)?;
                HidInput::type_text(&session.udid, &text)?;
            } else {
                hot_client()?.type_text(&text)?;
            }
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "chars": text.chars().count() }));
            } else {
                println!("✓ type {} chars via {}", text.chars().count(), if direct { "direct" } else { "lighd" });
            }
        }

        Commands::Ax => {
            let session = require_session(&config)?;
            let t0 = Instant::now();
            let dump = AxDump::dump(&session.udid)?;
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "ax_ms": ms, "tree": dump }));
            } else {
                let count = dump.get("element_count").and_then(|c| c.as_u64()).unwrap_or(0);
                println!("✓ ax dump — {count} elements in {ms:.1} ms");
                if let Some(els) = dump.get("elements").and_then(|e| e.as_array()) {
                    for el in els.iter().take(40) {
                        let label = el.get("label").and_then(|v| v.as_str()).unwrap_or("");
                        let role = el.get("role").and_then(|v| v.as_str()).unwrap_or("");
                        let id = el.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
                        println!("  [{role}] label={label:?} id={id:?}");
                    }
                    if els.len() > 40 {
                        println!("  … {} more", els.len() - 40);
                    }
                }
            }
        }

        Commands::Swipe { from_x, from_y, to_x, to_y, points } => {
            let normalized = !points;
            if direct {
                let session = require_session(&config)?;
                let (sim_w, sim_h) = session_dims(&session);
                let (fnx, fny, tnx, tny) = if points {
                    (from_x / sim_w, from_y / sim_h, to_x / sim_w, to_y / sim_h)
                } else {
                    (from_x, from_y, to_x, to_y)
                };
                HidInput::swipe(&session.udid, fnx, fny, tnx, tny, sim_w, sim_h)?;
            } else {
                hot_client()?.swipe(from_x, from_y, to_x, to_y, normalized)?;
            }
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "path": if direct { "direct" } else { "lighd" } }));
            } else {
                println!("✓ swipe via {}", if direct { "direct" } else { "lighd" });
            }
        }

        Commands::Home => {
            if direct {
                let session = require_session(&config)?;
                HidInput::home(&session.udid)?;
            } else {
                hot_client()?.home()?;
            }
            if use_json {
                println!("{}", serde_json::json!({ "ok": true, "path": if direct { "direct" } else { "lighd" } }));
            } else {
                println!("✓ home via {}", if direct { "direct" } else { "lighd" });
            }
        }

        Commands::Screenshot { output } => {
            let out_path = output.map(std::path::PathBuf::from).unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".ligh")
                    .join("screenshot.png")
            });
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            if direct {
                let session = require_session(&config)?;
                ensure_headless();
                let compositor = Arc::new(FrameCompositor::new()?);
                let comp = compositor.clone();
                HostSession::set_frame_handler(move |id, w, h| comp.ingest(id, w, h));
                let host = HostSession::stream_start(&session.udid)?;
                host.wait_for_frames(1, Duration::from_secs(10))?;
                HostSession::poll_stream();
                let shot = Screenshot::capture(&compositor)?;
                shot.write_png(&out_path)?;
                if use_json {
                    println!("{}", serde_json::json!({
                        "ok": true, "path": out_path.display().to_string(),
                        "width": shot.width, "height": shot.height, "via": "direct"
                    }));
                } else {
                    println!("✓ screenshot {}×{} → {} (direct)", shot.width, shot.height, out_path.display());
                }
            } else {
                let data = hot_client()?.screenshot(out_path.display().to_string())?;
                if use_json {
                    println!("{}", serde_json::json!({ "ok": true, "data": data, "via": "lighd" }));
                } else {
                    println!("✓ screenshot → {} (lighd)", out_path.display());
                }
            }
        }

        Commands::Observe {
            no_ax,
            settle_ms,
            no_settle,
        } => {
            if direct {
                let snap = observe_direct(&config, !no_ax)?;
                if use_json {
                    println!("{}", serde_json::to_string_pretty(&snap)?);
                } else {
                    print_observe(&snap);
                }
            } else {
                let settle = if no_settle { None } else { Some(settle_ms) };
                let data = hot_client()?.observe_ax_settle(!no_ax, settle)?;
                if use_json {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                } else if let Ok(snap) = serde_json::from_value::<ObserveSnapshot>(data.clone()) {
                    print_observe(&snap);
                } else {
                    println!("{data}");
                }
            }
        }

        Commands::Ready {
            settle_ms,
            recover_homes,
        } => {
            let client = hot_client()?;
            match client.ensure_ready(Some(settle_ms), Some(recover_homes)) {
                Ok(data) => {
                    if use_json {
                        println!("{}", serde_json::to_string_pretty(&data)?);
                    } else {
                        println!("✓ ready — {}", data.get("phase").and_then(|v| v.as_str()).unwrap_or("?"));
                    }
                }
                Err(e) => {
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": false, "error": e.to_string() }));
                        std::process::exit(2);
                    }
                    anyhow::bail!("{e}");
                }
            }
        }

        Commands::Cap { command } => {
            use ligh_core::DaemonRequest;
            let client = hot_client()?;
            let req = match command {
                CapCommands::OpenSettings { settle_ms } => DaemonRequest::OpenSettings {
                    settle_ms: Some(settle_ms),
                },
                CapCommands::SettingsSearch { query, settle_ms } => DaemonRequest::SettingsSearch {
                    query,
                    settle_ms: Some(settle_ms),
                },
                CapCommands::AssertSurface { surface, settle_ms } => DaemonRequest::AssertSurface {
                    surface,
                    settle_ms: Some(settle_ms),
                },
                CapCommands::Tap {
                    label,
                    id,
                    settle_ms,
                    timeout_ms,
                } => DaemonRequest::ActTap {
                    label,
                    id,
                    settle_ms: Some(settle_ms),
                    timeout_ms: Some(timeout_ms),
                },
                CapCommands::Type { text, settle_ms } => DaemonRequest::ActType {
                    text,
                    settle_ms: Some(settle_ms),
                },
                CapCommands::RunApp {
                    app,
                    bundle_id,
                    wait_label,
                    wait_id,
                    settle_ms,
                    timeout_ms,
                    no_install,
                    launch_args,
                } => DaemonRequest::RunApp {
                    app,
                    bundle_id,
                    wait_label,
                    wait_id,
                    settle_ms: Some(settle_ms),
                    timeout_ms: Some(timeout_ms),
                    install: Some(!no_install),
                    launch_args: if launch_args.is_empty() {
                        None
                    } else {
                        Some(launch_args)
                    },
                },
                CapCommands::WaitLabel {
                    label,
                    settle_ms,
                    timeout_ms,
                } => DaemonRequest::WaitLabel {
                    label,
                    settle_ms: Some(settle_ms),
                    timeout_ms: Some(timeout_ms),
                },
                CapCommands::AppJob {
                    app,
                    bundle_id,
                    steps,
                    settle_ms,
                    timeout_ms,
                    no_install,
                    launch_args,
                } => {
                    let parsed: Vec<serde_json::Value> = serde_json::from_str(&steps)
                        .map_err(|e| anyhow::anyhow!("--steps JSON: {e}"))?;
                    DaemonRequest::AppJob {
                        app,
                        bundle_id,
                        steps: parsed,
                        settle_ms: Some(settle_ms),
                        timeout_ms: Some(timeout_ms),
                        install: Some(!no_install),
                        launch_args: if launch_args.is_empty() {
                            None
                        } else {
                            Some(launch_args)
                        },
                    }
                },
            };
            let resp = client.call(&req)?;
            let data = resp.data.clone().unwrap_or(serde_json::json!({
                "ok": resp.ok,
                "error": resp.error,
            }));
            if use_json {
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                let fault = data.get("fault").and_then(|v| v.as_str()).unwrap_or("?");
                let cap = data.get("capability").and_then(|v| v.as_str()).unwrap_or("cap");
                if resp.ok {
                    println!("✓ {cap} fault={fault}");
                } else {
                    println!("✗ {cap} fault={fault} — {}", resp.error.unwrap_or_default());
                }
            }
            if !resp.ok {
                std::process::exit(2);
            }
        }

        Commands::Daemon { command } => match command {
            DaemonCommands::Start => {
                let client = hot_client()?;
                if use_json {
                    println!("{}", serde_json::json!({ "ok": true, "sock": client.sock_path().display().to_string() }));
                } else {
                    println!("✓ lighd up @ {}", client.sock_path().display());
                }
            }
            DaemonCommands::Status => {
                let client = DaemonClient::default_sock();
                let alive = client.is_alive();
                if use_json {
                    println!("{}", serde_json::json!({ "alive": alive, "sock": default_sock_path().display().to_string() }));
                } else if alive {
                    println!("✓ lighd alive @ {}", default_sock_path().display());
                } else {
                    println!("✗ lighd not running — try `ligh daemon start`");
                    std::process::exit(1);
                }
            }
            DaemonCommands::Stop => {
                let client = DaemonClient::default_sock();
                if !client.is_alive() {
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true, "already_stopped": true }));
                    } else {
                        println!("✓ lighd already stopped");
                    }
                } else {
                    // Quit daemon only — do not shut down the guest.
                    client.call(&ligh_core::DaemonRequest::Quit)?.into_result()?;
                    if use_json {
                        println!("{}", serde_json::json!({ "ok": true }));
                    } else {
                        println!("✓ lighd quit (guest left booted)");
                    }
                }
            }
        },

        Commands::AgentBench {
            iterations,
            vs_cold,
            no_cold,
            micro_only,
            workload_only,
            steps,
            no_wda,
        } => {
            let client = hot_client()?;
            agent_bench::run_agent_bench(
                &config,
                client,
                agent_bench::AgentBenchOpts {
                    iterations,
                    vs_cold: vs_cold && !no_cold,
                    micro_only,
                    workload_only,
                    steps,
                    use_json,
                    no_wda,
                },
            )?;
        }
    }
    Ok(())
}


// ────────────────────────── helpers ──────────────────────────────────────────

fn hot_client() -> anyhow::Result<DaemonClient> {
    Ok(ensure_daemon(&default_sock_path(), &sibling_lighd())?)
}

fn print_observe(snap: &ObserveSnapshot) {
    println!("schema:            {}", snap.schema_version);
    println!("udid:              {}", snap.udid);
    println!("booted:            {}", snap.booted);
    println!("ax_quality:        {} settled={}", snap.ax_quality, snap.settled);
    if let Some(scene) = &snap.scene {
        println!(
            "scene:             surface={:?} title={:?} kb={} alerts={}",
            scene.surface,
            scene.screen_title,
            scene.keyboard_visible,
            scene.alerts.len()
        );
    }
    if !snap.events.is_empty() {
        println!("events:            {} sense event(s)", snap.events.len());
        for e in snap.events.iter().take(6) {
            println!("  • {} {:?}", e.kind, e.payload);
        }
    }
    println!("actionable_topk:   {}", snap.actionable_topk.len());
    for n in snap.actionable_topk.iter().take(12) {
        let id = n.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let label = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let cn = n.get("center_norm");
        let cx = cn.and_then(|c| c.get("x")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cy = cn.and_then(|c| c.get("y")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        println!("  • [{role}] {label} id={id} @({cx:.2},{cy:.2})");
    }
    match &snap.accessibility_tree {
        AccessibilityTree::Available {
            nodes,
            element_count,
            ..
        } => {
            println!(
                "accessibility:     available ({} elements)",
                element_count.unwrap_or(nodes.len())
            );
        }
        AccessibilityTree::Empty => println!("accessibility:     empty"),
        AccessibilityTree::Error { message } => println!("accessibility:     error — {message}"),
        AccessibilityTree::NotImplemented => println!("accessibility:     not_implemented"),
    }
}

fn observe_direct(config: &LighConfig, include_ax: bool) -> anyhow::Result<ObserveSnapshot> {
    let t0 = Instant::now();
    let session = require_session(config)?;
    let booted = Simctl::is_booted(&session.udid).unwrap_or(false);
    let sim_app = std::process::Command::new("pgrep")
        .args(["-x", "Simulator"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let frame = if booted {
        match quick_frame_meta(&session.udid) {
            Ok(m) => Some(m),
            Err(_) => None,
        }
    } else {
        None
    };
    let ax = if include_ax {
        match AxDump::dump(&session.udid) {
            Ok(v) => AccessibilityTree::from_ax_dump(v),
            Err(e) => AccessibilityTree::Error {
                message: e.to_string(),
            },
        }
    } else {
        AccessibilityTree::Empty
    };
    let mut snap = ObserveSnapshot {
        schema_version: ligh_core::OBSERVE_SCHEMA_VERSION,
        udid: session.udid.clone(),
        booted,
        simulator_app_running: sim_app,
        frame,
        app_bundle_id: session.app_bundle_id.clone(),
        accessibility_tree: ax,
        scene: None,
        actionable_topk: vec![],
        events: vec![],
        ax_quality: "empty".into(),
        settled: false,
        observe_ms: Some(t0.elapsed().as_secs_f64() * 1000.0),
        path: Some("direct".into()),
        phase: None,
        eyes_unusable: false,
        overlay: None,
    };
    snap.enrich_v2();
    Ok(snap)
}

fn require_session(config: &LighConfig) -> anyhow::Result<SessionState> {
    SessionState::load(&config.state_dir)?
        .ok_or_else(|| anyhow::anyhow!("no session — run `ligh up` first"))
}

fn session_dims(session: &SessionState) -> (f64, f64) {
    session.device.logical_size()
}

/// Boot a short-lived IOSurface session, grab frame meta, clean up.
fn quick_frame_meta(udid: &str) -> anyhow::Result<FrameMeta> {
    ensure_headless();
    let compositor = Arc::new(FrameCompositor::new()?);
    let comp = compositor.clone();
    HostSession::set_frame_handler(move |id, w, h| comp.ingest(id, w, h));
    let _host = HostSession::stream_start(udid)?;
    // poll for up to 3s
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        HostSession::poll_stream();
        let s = compositor.stats();
        if s.imports_ok > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let s = compositor.stats();
    Ok(FrameMeta {
        width: s.last_width,
        height: s.last_height,
        id: s.imports_ok,
        fps: s.fps,
        imports_ok: s.imports_ok > 0,
    })
}

fn print_status_report(st: &ligh_sim::StatusReport) {
    println!("disk: {} MB", st.disk_free_mb);
    if let Some(s) = &st.session {
        println!("udid: {}", s.udid);
        println!("booted: {}", st.booted);
        if let Some(n) = st.disabled_at_boot {
            println!("disabled-at-boot jobs: {n}");
        }
        if let Some(fp) = &st.footprint {
            println!("RAM:  {:.0} MB ({} procs)", fp.total_mb, fp.process_count);
        }
    } else {
        println!("no session");
    }
}

fn print_bench(r: &ligh_sim::BenchReport) {
    println!();
    println!("┌──────────────────────┬────────────┬────────────┬────────────┐");
    println!("│                      │ Stock*     │ LIGH       │ Delta      │");
    println!("├──────────────────────┼────────────┼────────────┼────────────┤");
    println!("│ Boot to ready        │ {:>8.1}s │ {:>8.1}s │ {:>+8.1}s │",
        r.stock_boot_secs, r.ligh_boot_secs, r.ligh_boot_secs - r.stock_boot_secs);
    println!("│ Scoped RAM           │ {:>6.0} MB │ {:>6.0} MB │ {:>+6.0} MB │",
        r.stock_ram_mb, r.ligh_ram_mb, -r.ram_saved_mb);
    println!("│ Simulator.app host   │ {:>6.0} MB │ {:>6.0} MB │ {:>+6.0} MB │",
        r.stock_simulator_app_mb, r.ligh_simulator_app_mb, -r.host_saved_mb);
    println!("│ Process count        │ {:>10} │ {:>10} │            │",
        r.stock_procs, r.ligh_procs);
    println!("└──────────────────────┴────────────┴────────────┴────────────┘");
    println!("* Stock = simctl boot + Simulator.app open");
    println!("→ {:.0}% scoped RAM delta · host GUI saved {:.0} MB (Simulator.app stack)",
        r.ram_saved_pct, r.host_saved_mb);
}

fn init_tracing(verbose: bool) {
    let d = if verbose { "ligh=debug" } else { "ligh=info" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(d)))
        .try_init();
}
