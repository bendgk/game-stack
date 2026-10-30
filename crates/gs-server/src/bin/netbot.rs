//! netbot — headless network players over real WebRTC (str0m playing the browser's role).
//!
//! Each bot does exactly what the web client does: POST an SDP offer, open `rel` + `unrel`
//! DataChannels over UDP, send `Hello`, then run the real `Predictor` with the game's bot policy
//! as its "hands". Use it to smoke-test a server, fill a room, or load-test the netcode.
//!
//! ```sh
//! cargo run -p gs-server --bin netbot -- --server http://127.0.0.1:8080 --room test --clients 4 --seconds 20
//! ```

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use gs_core::Game;
use gs_core::net::Channel;
use gs_core::net::client::{Predictor, decode_welcome, hello};
use str0m::change::SdpAnswer;
use str0m::channel::{ChannelConfig, ChannelId, Reliability};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

type G = game::TheGame;

struct Args {
    server: String,
    room: String,
    mode: String,
    clients: usize,
    seconds: f64,
    bots: u8,
}

fn args() -> Args {
    let mut a = Args {
        server: "http://127.0.0.1:8080".into(),
        room: "netbot".into(),
        mode: "versus".into(),
        clients: 2,
        seconds: 10.0,
        bots: 0,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i + 1 < v.len() {
        let val = v[i + 1].clone();
        match v[i].as_str() {
            "--server" => a.server = val,
            "--room" => a.room = val,
            "--mode" => a.mode = val,
            "--clients" => a.clients = val.parse().unwrap_or(2),
            "--seconds" => a.seconds = val.parse().unwrap_or(10.0),
            "--bots" => a.bots = val.parse().unwrap_or(0),
            _ => {}
        }
        i += 2;
    }
    a
}

/// Minimal HTTP/1.1 POST (keeps this tool dependency-free).
async fn post_json(base: &str, path: &str, body: &str) -> anyhow::Result<String> {
    let hostport = base.trim_start_matches("http://").trim_end_matches('/');
    let mut s = TcpStream::connect(hostport).await?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).await?;
    let text = String::from_utf8_lossy(&resp).to_string();
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| anyhow::anyhow!("bad http response"))?;
    if !head.starts_with("HTTP/1.1 200") {
        anyhow::bail!("signaling failed: {head} {body}");
    }
    Ok(body.to_string())
}

fn lan_ip() -> IpAddr {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.0.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]))
}

#[derive(Debug, Default)]
struct Report {
    welcomed: bool,
    snapshots: u32,
    rollbacks: u32,
    hits: u32,
    rtt_ms: f32,
    slack: f32,
    events: usize,
    error: Option<String>,
}

async fn run_bot(idx: usize, a: &Args) -> anyhow::Result<Report> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 0)).await?;
    let local = SocketAddr::new(lan_ip(), socket.local_addr()?.port());
    let mut rtc = Rtc::builder().build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local, "udp")?);
    let mut api = rtc.sdp_api();
    let rel = api.add_channel("rel".into());
    let unrel = api.add_channel_with_config(ChannelConfig {
        label: "unrel".into(),
        ordered: false,
        reliability: Reliability::MaxRetransmits { retransmits: 0 },
        ..Default::default()
    });
    let (offer, pending) = api.apply().ok_or_else(|| anyhow::anyhow!("no offer"))?;
    let answer_json = post_json(&a.server, "/api/rtc/offer", &serde_json::to_string(&offer)?).await?;
    let answer: SdpAnswer = serde_json::from_str(&answer_json)?;
    rtc.sdp_api().accept_answer(pending, answer)?;

    let channels: [ChannelId; 2] = [rel, unrel];
    let mut open = [false; 2];
    let mut sent_hello = false;
    let mut pred: Option<Predictor<G>> = None;
    let mut report = Report::default();
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(a.seconds);
    let mut buf = vec![0u8; 2048];
    let mut next_frame = Instant::now();
    let now_ms = |t: Instant| t.duration_since(start).as_secs_f64() * 1000.0;

    while Instant::now() < deadline {
        // Drain str0m.
        let timeout = loop {
            match rtc.poll_output()? {
                Output::Timeout(t) => break t,
                Output::Transmit(t) => {
                    socket.send_to(&t.contents, t.destination).await?;
                }
                Output::Event(Event::ChannelOpen(id, _)) => {
                    if let Some(i) = channels.iter().position(|c| *c == id) {
                        open[i] = true;
                    }
                }
                Output::Event(Event::ChannelData(d)) => match &mut pred {
                    None => {
                        if let Some(w) = decode_welcome(&d.data)? {
                            report.welcomed = true;
                            pred = Some(Predictor::new(w)?);
                        }
                    }
                    Some(p) => p.receive(&d.data, now_ms(Instant::now()))?,
                },
                Output::Event(_) => {}
            }
        };
        if open == [true, true] && !sent_hello {
            sent_hello = true;
            let h = hello::<G>(&format!("netbot-{idx}"), &a.room, &a.mode, a.bots, None);
            if let Some(mut ch) = rtc.channel(rel) {
                ch.write(true, &h)?;
            }
        }
        // ~60 fps "display" loop driving the predictor with the game's own bot policy.
        let now = Instant::now();
        if now >= next_frame {
            next_frame = now + Duration::from_micros(16_667);
            if let Some(p) = &mut pred {
                let input = p.game().bot_input(p.me());
                p.set_input(input);
                p.update(now_ms(now));
                for (ch, bytes) in p.drain_outbox().collect::<Vec<_>>() {
                    let id = if ch == Channel::Reliable { rel } else { unrel };
                    if let Some(mut c) = rtc.channel(id) {
                        let _ = c.write(true, &bytes);
                    }
                }
                report.events += p.drain_events().count();
                p.drain_notices().for_each(|n| eprintln!("[netbot-{idx}] notice: {n}"));
            }
            continue;
        }
        let wait = timeout.min(next_frame).saturating_duration_since(Instant::now()).max(Duration::from_micros(100));
        match tokio::time::timeout(wait, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, source))) => {
                let Ok(contents) = buf[..n].try_into() else { continue };
                rtc.handle_input(Input::Receive(
                    Instant::now(),
                    Receive { proto: Protocol::Udp, source, destination: local, contents },
                ))?;
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => rtc.handle_input(Input::Timeout(Instant::now()))?,
        }
    }
    if let Some(p) = &pred {
        let s = p.stats();
        report.snapshots = s.snapshots;
        report.rollbacks = s.rollbacks;
        report.hits = s.prediction_hits;
        report.rtt_ms = s.rtt_ms;
        report.slack = s.slack;
    }
    Ok(report)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    str0m::crypto::from_feature_flags().install_process_default();
    let a = std::sync::Arc::new(args());
    eprintln!("netbot: {} client(s) → {} room {:?} for {}s", a.clients, a.server, a.room, a.seconds);
    let mut handles = Vec::new();
    for i in 0..a.clients {
        let a = a.clone();
        handles.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(i as u64 * 150)).await;
            run_bot(i, &a).await.unwrap_or_else(|e| Report { error: Some(format!("{e:#}")), ..Default::default() })
        }));
    }
    let mut ok = true;
    let expected = (a.seconds * f64::from(G::TICK_HZ) * 0.5) as u32;
    for (i, h) in handles.into_iter().enumerate() {
        let r = h.await?;
        let pass = r.error.is_none() && r.welcomed && r.snapshots >= expected;
        ok &= pass;
        println!(
            "netbot-{i}: {} welcomed={} snapshots={} rollbacks={} hits={} rtt={:.1}ms slack={:.2} events={} {}",
            if pass { "OK " } else { "FAIL" },
            r.welcomed,
            r.snapshots,
            r.rollbacks,
            r.hits,
            r.rtt_ms,
            r.slack,
            r.events,
            r.error.unwrap_or_default()
        );
    }
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
