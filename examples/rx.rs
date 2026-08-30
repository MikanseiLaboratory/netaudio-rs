//! Dante 互換の受信デバイスを起動する CLI です。
//!
//! Windows / macOS / Linux で同じコマンドです。パッチ後の PCM は
//! OSのdefault出力で再生し、標準出力にピークを出します。
//!
//! ```text
//! cargo run --example rx --features cpal -- ifaces
//! cargo run --example rx --features cpal -- listen --bind 192.168.1.10 --name studio-rx --rx-channels 2
//! cargo run --example rx --features cpal -- listen --bind 192.168.1.10 --no-play
//! ```
//!
//! ログは `RUST_LOG=debug` で増やせます。Ctrl+C で停止します。

use clap::{Parser, Subcommand, ValueEnum};
use netaudio::{
    AudioFrameMut, Bind, Bits, BoundPorts, ClockStatus, Device, MediaTime, Sample, Settings,
};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// netaudio 受信 CLI。
#[derive(Debug, Parser)]
#[command(
    name = "rx",
    version,
    about = "Dante 互換の受信デバイスとして起動します。",
    long_about = "プロセスが Dante 機器として見え、Controller からのパッチを受けます。\
                  既定ではOSのdefault出力で再生し、1秒ごとにピークを表示します。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// IPv4 を持つネットワークインタフェースを一覧します。
    Ifaces,
    /// 受信デバイスを起動します。
    Listen(ListenArgs),
}

#[derive(Debug, clap::Args)]
struct ListenArgs {
    /// バインド先。IPv4 またはインタフェース名（Windows の表示名も可）。
    #[arg(long)]
    bind: String,

    /// Dante Controller に出るホスト名（DNS ラベル、1..=31 文字）。
    #[arg(long, default_value = "netaudio")]
    name: String,

    /// 受信チャネル数。
    #[arg(long, default_value_t = 2)]
    rx_channels: u16,

    /// サンプルレート。44100 / 48000 / 88200 / 96000。
    #[arg(long, default_value_t = 48_000, value_parser = parse_sample_rate)]
    sample_rate: u32,

    /// ワイヤ上のビット深度。
    #[arg(long, value_enum, default_value_t = BitDepth::B24)]
    bits: BitDepth,

    /// 受信レイテンシ（ミリ秒）。下限は 4。
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(4..=1000))]
    latency_ms: u64,

    /// 制御ポートの基準。未指定時は 4440 / 8800 / 4455 / 8700。埋まっていれば空いている4連を選びます。
    #[arg(long)]
    alt_port: Option<u16>,

    /// OSのdefault出力で再生します（既定で有効）。
    #[arg(long, action = clap::ArgAction::SetTrue)]
    play: bool,

    /// 再生せず、ピーク表示のみにします。
    #[arg(long = "no-play", action = clap::ArgAction::SetTrue)]
    no_play: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BitDepth {
    #[value(name = "16")]
    B16,
    #[value(name = "24")]
    B24,
    #[value(name = "32")]
    B32,
}

impl From<BitDepth> for Bits {
    fn from(value: BitDepth) -> Self {
        match value {
            BitDepth::B16 => Bits::B16,
            BitDepth::B24 => Bits::B24,
            BitDepth::B32 => Bits::B32,
        }
    }
}

fn parse_sample_rate(raw: &str) -> Result<u32, String> {
    const ALLOWED: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
    let rate: u32 = raw
        .parse()
        .map_err(|_| format!("sample-rate は整数で指定してください: {raw}"))?;
    if ALLOWED.contains(&rate) {
        Ok(rate)
    } else {
        Err(format!(
            "sample-rate は {} のいずれかです",
            ALLOWED
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(" / ")
        ))
    }
}

/// `--bind` がドット付き IPv4 ならアドレス、それ以外はインタフェース名です。
fn parse_bind(raw: &str) -> Bind {
    match raw.parse::<Ipv4Addr>() {
        Ok(ip) => Bind::Ip(ip),
        Err(_) => Bind::Interface(raw.to_owned()),
    }
}

fn settings_from_args(args: &ListenArgs) -> Settings {
    let latency = Duration::from_millis(args.latency_ms);
    Settings {
        name: args.name.clone(),
        bind: parse_bind(&args.bind),
        rx_channels: args.rx_channels,
        sample_rate: args.sample_rate,
        bits: Bits::from(args.bits),
        rx_latency: latency,
        tx_latency: latency,
        alt_port: args.alt_port,
        ..Default::default()
    }
}

fn print_ifaces() {
    println!("{:<24} {:<24} IPv4", "NAME", "DISPLAY");
    for iface in netdev::get_interfaces() {
        let display = iface
            .friendly_name
            .clone()
            .or(iface.description.clone())
            .unwrap_or_default();
        if iface.ipv4.is_empty() {
            continue;
        }
        for net in &iface.ipv4 {
            let ip = net.addr();
            if ip.is_unspecified() {
                continue;
            }
            println!("{:<24} {:<24} {ip}", iface.name, display);
        }
    }
}

fn print_ports(name: &str, bind: &str, ports: &BoundPorts) {
    println!("device  {name}");
    println!("bind    {bind}");
    println!("ARC     UDP {}", ports.arc);
    println!("CMC     UDP {}", ports.cmc);
    println!("info    UDP {}", ports.info);
    if let Some(p) = ports.mdns {
        println!("mDNS    UDP {p}");
    }
    if let Some(p) = ports.ptp_event {
        println!("PTP     UDP {p} / {}", ports.ptp_general.unwrap_or(0));
    }
    if ports.media.is_empty() {
        println!("media   パッチ後にUDP 14336..=14591へbindします");
    } else {
        println!("media   {:?}", ports.media);
    }
}

fn clock_label(status: ClockStatus) -> &'static str {
    match status {
        ClockStatus::Unlocked => "unlocked",
        ClockStatus::MediaDriven => "media",
        ClockStatus::PtpLocked => "ptp",
        _ => "unknown",
    }
}

/// 左詰め i32 のピークを dBFS にします。無音は `-inf` です。
fn peak_dbfs(peak: Sample) -> String {
    if peak == 0 {
        return "-inf".into();
    }
    let fs = 2147483648.0_f64;
    let db = 20.0 * (peak.unsigned_abs() as f64 / fs).log10();
    format!("{db:6.1}")
}

fn print_meter_line(status: ClockStatus, frames: u64, peaks: &[Sample]) {
    let db: Vec<String> = peaks.iter().copied().map(peak_dbfs).collect();
    println!(
        "clock={:<6} frames={:<7} peak_dBFS=[{}]",
        clock_label(status),
        frames,
        db.join(" ")
    );
}

async fn listen(args: ListenArgs) -> Result<(), netaudio::Error> {
    let want_play = match (args.play, args.no_play) {
        (_, true) => false,
        (true, false) | (false, false) => true,
    };
    let settings = settings_from_args(&args);
    let channels = settings.rx_channels;
    let device = Arc::new(Device::start(settings).await?);
    print_ports(&args.name, &args.bind, &device.bound_ports());
    println!("handshake 0x0100をメディアUDPから送ります（src=media）");

    let output = if want_play {
        match netaudio::play::Output::start_default(Arc::clone(&device)) {
            Ok(out) => {
                println!(
                    "play    {} ({} Hz, {} ch)",
                    out.device_name(),
                    out.sample_rate(),
                    out.channels()
                );
                Some(out)
            }
            Err(err) => {
                eprintln!(
                    "warning: default outputを開けませんでした ({err})。ピーク表示のみ続けます。"
                );
                None
            }
        }
    } else {
        println!("play    off");
        None
    };
    println!("Ctrl+C で停止します。");

    let playing = output.is_some();
    let mut buf = vec![0i32; 64 * channels as usize];
    let mut peaks = vec![0i32; channels as usize];
    let mut frames_in_window = 0u64;
    let mut last_report = Instant::now();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!();
                println!("stopping…");
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if playing {
                    if last_report.elapsed() >= Duration::from_secs(1)
                        && let Some(out) = output.as_ref()
                    {
                        let (frames, snap) = out.snapshot_meters();
                        print_meter_line(device.clock_status(), frames, &snap);
                        last_report = Instant::now();
                    }
                    continue;
                }
                let mut frame = AudioFrameMut {
                    media_time: MediaTime {
                        sample_index: 0,
                        ns: 0,
                    },
                    sample_rate: 0,
                    channels: 0,
                    samples: &mut buf,
                };
                let n = device.try_read(&mut frame)?;
                if n > 0 {
                    frames_in_window += n as u64;
                    for frame_i in 0..n {
                        for ch in 0..channels as usize {
                            let s = buf[frame_i * channels as usize + ch];
                            peaks[ch] = peaks[ch].max(s.saturating_abs());
                        }
                    }
                }
                if last_report.elapsed() >= Duration::from_secs(1) {
                    print_meter_line(device.clock_status(), frames_in_window, &peaks);
                    frames_in_window = 0;
                    peaks.fill(0);
                    last_report = Instant::now();
                }
            }
        }
    }

    drop(output);
    match Arc::try_unwrap(device) {
        Ok(dev) => dev.shutdown().await,
        Err(_) => Ok(()),
    }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ifaces => {
            print_ifaces();
            Ok(())
        }
        Command::Listen(args) => listen(args).await,
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        if matches!(err, netaudio::Error::PortInUse { .. }) {
            eprintln!(
                "hint: WindowsではHyper-VがUDPを予約することがあります。`netsh interface ipv4 show excludedportrange protocol=udp` を見て `--alt-port`でずらしてください。"
            );
        }
        std::process::exit(1);
    }
}
