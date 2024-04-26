#![cfg(target_arch = "wasm32")]
use anyhow::{Context, Result};
use libp2p::{
    core::Multiaddr,
    futures::{AsyncReadExt, AsyncWriteExt, StreamExt},
    multiaddr::Protocol,
    ping,
    swarm::SwarmEvent,
    PeerId, Stream, StreamProtocol,
};
use libp2p_stream as stream;
use libp2p_webrtc_websys as webrtc_websys;
use rand::RngCore;
use std::io;
use std::time::Duration;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement};

mod conventions;
use conventions::a;

const ECHO_PROTOCOL: StreamProtocol = StreamProtocol::new("/echo");

#[wasm_bindgen]
pub async fn run(libp2p_endpoint: String) -> Result<(), JsError> {
    tracing_wasm::set_as_global_default();

    let ping_duration = Duration::from_secs(30);

    let body = Body::from_current_window()?;
    body.append_p(&format!(
        "Let's ping the rust-libp2p server over WebRTC for {:?}:",
        ping_duration
    ))?;

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_wasm_bindgen()
        .with_other_transport(|key| {
            webrtc_websys::Transport::new(webrtc_websys::Config::new(&key))
        })?
        .with_behaviour(|_| ping::Behaviour::new(ping::Config::new()))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(ping_duration))
        .build();

    let addr = libp2p_endpoint.parse::<Multiaddr>()?;
    tracing::info!("Dialing {addr}");
    swarm.dial(addr)?;

    loop {
        match swarm.next().await.unwrap() {
            SwarmEvent::Behaviour(ping::Event { result: Err(e), .. }) => {
                tracing::error!("Ping failed: {:?}", e);

                break;
            }
            SwarmEvent::Behaviour(ping::Event {
                peer,
                result: Ok(rtt),
                ..
            }) => {
                tracing::info!("Ping successful: RTT: {rtt:?}, from {peer}");
                body.append_p(&format!("RTT: {rtt:?}"))?;
            }
            SwarmEvent::ConnectionClosed {
                cause: Some(cause), ..
            } => {
                tracing::info!("Swarm event: {:?}", cause);

                if let libp2p::swarm::ConnectionError::KeepAliveTimeout = cause {
                    body.append_p("All done with pinging! ")?;

                    break;
                }
                body.append_p(&format!("Connection closed due to: {:?}", cause))?;
            }
            evt => tracing::info!("Swarm event: {:?}", evt),
        }
    }

    Ok(())
}

/// Convenience wrapper around the current document body
struct Body {
    body: HtmlElement,
    document: Document,
}

impl Body {
    fn from_current_window() -> Result<Self, JsError> {
        // Use `web_sys`'s global `window` function to get a handle on the global
        // window object.
        let document = web_sys::window()
            .ok_or(js_error("no global `window` exists"))?
            .document()
            .ok_or(js_error("should have a document on window"))?;
        let body = document
            .body()
            .ok_or(js_error("document should have a body"))?;

        Ok(Self { body, document })
    }

    fn append_p(&self, msg: &str) -> Result<(), JsError> {
        let val = self
            .document
            .create_element("p")
            .map_err(|_| js_error("failed to create <p>"))?;
        val.set_text_content(Some(msg));
        self.body
            .append_child(&val)
            .map_err(|_| js_error("failed to append <p>"))?;

        Ok(())
    }
}

fn js_error(msg: &str) -> JsError {
    io::Error::new(io::ErrorKind::Other, msg).into()
}

/// A very simple, `async fn`-based connection handler for our custom echo protocol.
async fn connection_handler(peer: PeerId, mut control: stream::Control) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await; // Wait a second between echos.

        let stream = match control.open_stream(peer, ECHO_PROTOCOL).await {
            Ok(stream) => stream,
            Err(error @ stream::OpenStreamError::UnsupportedProtocol(_)) => {
                tracing::info!(%peer, %error);
                return;
            }
            Err(error) => {
                // Other errors may be temporary.
                // In production, something like an exponential backoff / circuit-breaker may be more appropriate.
                tracing::debug!(%peer, %error);
                continue;
            }
        };

        if let Err(e) = send(stream).await {
            tracing::warn!(%peer, "Echo protocol failed: {e}");
            continue;
        }

        tracing::info!(%peer, "Echo complete!")
    }
}

async fn echo(mut stream: Stream) -> io::Result<usize> {
    let mut total = 0;

    let mut buf = [0u8; 100];

    loop {
        let read = stream.read(&mut buf).await?;
        if read == 0 {
            return Ok(total);
        }

        total += read;
        stream.write_all(&buf[..read]).await?;
    }
}

async fn send(mut stream: Stream) -> io::Result<()> {
    let num_bytes = rand::random::<usize>() % 1000;

    let mut bytes = vec![0; num_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);

    stream.write_all(&bytes).await?;

    let mut buf = vec![0; num_bytes];
    stream.read_exact(&mut buf).await?;

    if bytes != buf {
        return Err(io::Error::new(io::ErrorKind::Other, "incorrect echo"));
    }

    stream.close().await?;

    Ok(())
}
