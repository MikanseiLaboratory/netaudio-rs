//! Tokio CMC server (UDP 8800 or alt+1).

use super::{Error, Shared};
use crate::protocol::cmc;
use crate::protocol::req_resp;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::watch;

pub async fn run(
    shared: Arc<Shared>,
    sock: UdpSocket,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut buf = [0u8; 2048];
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            rec = sock.recv_from(&mut buf) => {
                match rec {
                    Ok((n, src)) => {
                        if let Some(reply) = handle(&shared, &buf[..n]) {
                            let _ = sock.send_to(&reply, src).await;
                        }
                    }
                    Err(e) => log::warn!("cmc recv: {e}"),
                }
            }
        }
    }
    Ok(())
}

fn handle(shared: &Shared, packet: &[u8]) -> Option<Vec<u8>> {
    let (hdr, _) = req_resp::decode(packet)?;
    match hdr.opcode1 {
        cmc::OP_ADVERTISEMENT => Some(cmc::encode_advertisement(
            hdr,
            shared.identity.process_id,
            shared.identity.device_id,
            shared.identity.ip.octets(),
            shared.identity.info_port,
        )),
        other => {
            log::debug!("cmc unknown opcode1={other:#06x}");
            None
        }
    }
}
