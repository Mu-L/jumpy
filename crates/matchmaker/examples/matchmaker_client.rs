use std::{net::SocketAddr, sync::Arc, time::Duration};

use certs::SkipServerVerification;
use jumpy_matchmaker_proto::{MatchInfo, MatchmakerRequest, MatchmakerResponse};
use once_cell::sync::Lazy;
use quinn::{ClientConfig, Endpoint, EndpointConfig};
use quinn_smol::SmolExecutor;

static EXE: Lazy<SmolExecutor> =
    Lazy::new(|| SmolExecutor(Arc::new(async_executor::Executor::default())));

static SERVER_NAME: &str = "localhost";

fn client_addr() -> SocketAddr {
    "127.0.0.1:0".parse::<SocketAddr>().unwrap()
}

fn server_addr() -> SocketAddr {
    "127.0.0.1:8943".parse::<SocketAddr>().unwrap()
}

mod certs {
    use std::sync::Arc;

    // Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
    pub struct SkipServerVerification;

    impl SkipServerVerification {
        pub fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    impl rustls::client::ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::Certificate,
            _intermediates: &[rustls::Certificate],
            _server_name: &rustls::ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: std::time::SystemTime,
        ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::ServerCertVerified::assertion())
        }
    }
}

fn configure_client() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    ClientConfig::new(Arc::new(crypto))
}

pub fn main() {
    futures_lite::future::block_on(EXE.run(async move {
        if let Err(e) = client().await {
            eprintln!("Error: {e}");
        }
    }));
}

async fn client() -> anyhow::Result<()> {
    let client_config = configure_client();
    let socket = std::net::UdpSocket::bind(client_addr())?;
    // Bind this endpoint to a UDP socket on the given client address.
    let endpoint = Endpoint::new(EndpointConfig::default(), None, socket, EXE.clone())?.0;

    println!("Opened client on {}", endpoint.local_addr()?);

    // Connect to the server passing in the server name which is supposed to be in the server certificate.
    let conn = endpoint
        .connect_with(client_config, server_addr(), SERVER_NAME)?
        .await?;

    // Send a match request to the server
    let (mut send, recv) = conn.open_bi().await?;

    let message = MatchmakerRequest::RequestMatch(MatchInfo { player_count: 1 });
    println!("Sending match request: {message:?}");
    let message = postcard::to_allocvec(&message)?;

    send.write_all(&message).await?;
    send.finish().await?;

    let message = recv.read_to_end(256).await?;
    let message: MatchmakerResponse = postcard::from_bytes(&message)?;

    match message {
        MatchmakerResponse::Success => {
            println!("Found a match!");
        }
    }

    async_io::Timer::after(Duration::from_secs(4)).await;

    conn.close(0u8.into(), b"done");

    endpoint.close(0u8.into(), b"done");
    endpoint.wait_idle().await;

    Ok(())
}