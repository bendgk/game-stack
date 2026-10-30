//! WebRTC transport: one UDP socket, one sans-IO `str0m::Rtc` per browser, one task.
//!
//! Signaling is a single HTTP POST (offer → answer). After that, every byte of game traffic is
//! SCTP-over-DTLS-over-UDP on two DataChannels the browser opens (`rel`, `unrel`).
//! The server is ICE-lite with a single host candidate (`GS_PUBLIC_IP:GS_UDP_PORT`): on a LAN
//! there is no NAT to traverse, and browsers that hide their own host candidates behind mDNS
//! still connect, because we learn their address from their first STUN packet.
//!
//! The str0m contract: mutate (input) → drain `poll_output` until `Timeout` → mutate again.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gs_core::net::Channel;
use str0m::change::{SdpAnswer, SdpOffer};
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

pub type PeerId = u64;

/// Unreliable sends are dropped (newest wins) once this much is queued on a channel.
const MAX_BUFFERED_UNRELIABLE: usize = 64 * 1024;

pub enum RtcEvent {
    /// Both DataChannels are open.
    Open(PeerId),
    Data(PeerId, Channel, Vec<u8>),
    Closed(PeerId),
}

#[allow(dead_code)] // `Close` is constructed by `RtcHandle::close` (extension API)
enum Cmd {
    Accept { offer: SdpOffer, reply: oneshot::Sender<anyhow::Result<SdpAnswer>> },
    Send { peer: PeerId, channel: Channel, data: Vec<u8> },
    Close { peer: PeerId },
}

#[derive(Clone)]
pub struct RtcHandle {
    tx: mpsc::UnboundedSender<Cmd>,
    peers: Arc<AtomicUsize>,
}

impl RtcHandle {
    /// Accept a browser's SDP offer; returns the answer for the HTTP response.
    pub async fn accept(&self, offer: SdpOffer) -> anyhow::Result<SdpAnswer> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Cmd::Accept { offer, reply }).map_err(|_| anyhow::anyhow!("rtc task gone"))?;
        rx.await?
    }
    pub fn send(&self, peer: PeerId, channel: Channel, data: Vec<u8>) {
        let _ = self.tx.send(Cmd::Send { peer, channel, data });
    }
    /// Drop a peer (kick). Part of the API for your own business logic.
    #[allow(dead_code)]
    pub fn close(&self, peer: PeerId) {
        let _ = self.tx.send(Cmd::Close { peer });
    }
    pub fn peer_count(&self) -> usize {
        self.peers.load(Ordering::Relaxed)
    }
}

struct Peer {
    rtc: Rtc,
    channels: [Option<ChannelId>; 2],
    opened: bool,
}

impl Peer {
    fn channel_of(&self, id: ChannelId) -> Option<Channel> {
        if self.channels[0] == Some(id) {
            Some(Channel::Reliable)
        } else if self.channels[1] == Some(id) {
            Some(Channel::Unreliable)
        } else {
            None
        }
    }
}

/// Bind the UDP socket and spawn the transport task.
pub async fn start(
    bind: SocketAddr,
    advertise: SocketAddr,
) -> anyhow::Result<(RtcHandle, mpsc::UnboundedReceiver<RtcEvent>)> {
    str0m::crypto::from_feature_flags().install_process_default();
    let socket = UdpSocket::bind(bind).await?;
    tracing::info!(%bind, %advertise, "webrtc udp socket bound");
    let (tx, rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let peers = Arc::new(AtomicUsize::new(0));
    tokio::spawn(run(socket, advertise, rx, ev_tx, peers.clone()));
    Ok((RtcHandle { tx, peers }, ev_rx))
}

async fn run(
    socket: UdpSocket,
    advertise: SocketAddr,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    events: mpsc::UnboundedSender<RtcEvent>,
    count: Arc<AtomicUsize>,
) {
    let mut peers: HashMap<PeerId, Peer> = HashMap::new();
    let mut next_id: PeerId = 1;
    let mut buf = vec![0u8; 2048];
    let mut deadline = Instant::now();

    loop {
        // Drive every peer: drain output until each reports its next timeout.
        let now = Instant::now();
        let mut next = now + Duration::from_millis(100);
        let mut dead = Vec::new();
        for (id, peer) in peers.iter_mut() {
            if now >= deadline {
                let _ = peer.rtc.handle_input(Input::Timeout(now));
            }
            match drain(*id, peer, &socket, &events).await {
                Some(t) => next = next.min(t),
                None => dead.push(*id),
            }
        }
        for id in dead {
            peers.remove(&id);
            let _ = events.send(RtcEvent::Closed(id));
            tracing::info!(peer = id, "peer closed");
        }
        count.store(peers.len(), Ordering::Relaxed);
        deadline = next;

        let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(
            deadline.max(Instant::now() + Duration::from_micros(200)),
        ));
        tokio::select! {
            biased;
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                handle_cmd(cmd, &mut peers, &mut next_id, advertise);
                // Commands often come in bursts (a snapshot per player per tick): take them all.
                while let Ok(cmd) = cmds.try_recv() {
                    handle_cmd(cmd, &mut peers, &mut next_id, advertise);
                }
            }
            res = socket.recv_from(&mut buf) => {
                let Ok((n, source)) = res else { continue };
                let Ok(contents) = buf[..n].try_into() else { continue };
                // In Docker the socket's local address differs from the advertised one; str0m
                // matches packets against the candidate it gave out, so use that.
                let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: advertise, contents });
                if let Some(peer) = peers.values_mut().find(|p| p.rtc.accepts(&input))
                    && let Err(e) = peer.rtc.handle_input(input) {
                        tracing::debug!("rtc input error: {e}");
                        peer.rtc.disconnect();
                    }
            }
            _ = sleep => {
                deadline = Instant::now();
            }
        }
    }
}

fn handle_cmd(cmd: Cmd, peers: &mut HashMap<PeerId, Peer>, next_id: &mut PeerId, advertise: SocketAddr) {
    match cmd {
        Cmd::Accept { offer, reply } => {
            let mut rtc = Rtc::builder().set_ice_lite(true).build(Instant::now());
            let res = (|| {
                let cand = Candidate::host(advertise, "udp")?;
                rtc.add_local_candidate(cand);
                Ok::<_, anyhow::Error>(rtc.sdp_api().accept_offer(offer)?)
            })();
            match res {
                Ok(answer) => {
                    let id = *next_id;
                    *next_id += 1;
                    peers.insert(id, Peer { rtc, channels: [None, None], opened: false });
                    tracing::info!(peer = id, "accepted webrtc offer");
                    let _ = reply.send(Ok(answer));
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
        }
        Cmd::Send { peer, channel, data } => {
            let Some(p) = peers.get_mut(&peer) else { return };
            let Some(cid) = p.channels[channel as usize] else { return };
            let Some(mut ch) = p.rtc.channel(cid) else { return };
            if channel == Channel::Unreliable && ch.buffered_amount() > MAX_BUFFERED_UNRELIABLE {
                return; // congested: the next snapshot supersedes this one
            }
            if let Err(e) = ch.write(true, &data) {
                tracing::debug!(peer, "channel write failed: {e}");
            }
        }
        Cmd::Close { peer } => {
            if let Some(p) = peers.get_mut(&peer) {
                p.rtc.disconnect();
            }
        }
    }
}

/// Poll a peer until it yields a timeout. `None` = the peer is dead.
async fn drain(
    id: PeerId,
    peer: &mut Peer,
    socket: &UdpSocket,
    events: &mpsc::UnboundedSender<RtcEvent>,
) -> Option<Instant> {
    loop {
        if !peer.rtc.is_alive() {
            return None;
        }
        match peer.rtc.poll_output() {
            Ok(Output::Timeout(t)) => return Some(t),
            Ok(Output::Transmit(t)) => {
                let _ = socket.send_to(&t.contents, t.destination).await;
            }
            Ok(Output::Event(e)) => match e {
                Event::ChannelOpen(cid, label) => {
                    if let Some(ch) = Channel::from_label(&label) {
                        peer.channels[ch as usize] = Some(cid);
                    }
                    if !peer.opened && peer.channels.iter().all(Option::is_some) {
                        peer.opened = true;
                        let _ = events.send(RtcEvent::Open(id));
                    }
                }
                Event::ChannelData(d) => {
                    if let Some(ch) = peer.channel_of(d.id) {
                        let _ = events.send(RtcEvent::Data(id, ch, d.data));
                    }
                }
                Event::ChannelClose(_) => peer.rtc.disconnect(),
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => peer.rtc.disconnect(),
                _ => {}
            },
            Err(e) => {
                tracing::debug!(peer = id, "rtc error: {e}");
                peer.rtc.disconnect();
                return None;
            }
        }
    }
}
