use std::{net::ToSocketAddrs, str::FromStr, time::Duration};

use rocket::{serde::json::Json, State};
use serde::Serialize;
use trust_dns_resolver::TokioAsyncResolver;

#[derive(Serialize)]
struct BedrockParsed {
    edition: String,
    motd1: String,
    motd2: String,
    pvn: u32,
    version: String,
    player_count: u32,
    max_player_count: u32,
    server_id: i128,
    gamemode: String,
    gamemode_num: u8,
    port: i32,
    port6: i32,
}

impl FromStr for BedrockParsed {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let mut iter = s.split(';');
        Ok(Self {
            edition: iter.next().ok_or(())?.to_string(),
            motd1: iter.next().ok_or(())?.to_string(),
            pvn: iter.next().ok_or(())?.parse().map_err(drop)?,
            version: iter.next().ok_or(())?.to_string(),
            player_count: iter.next().ok_or(())?.parse().map_err(drop)?,
            max_player_count: iter.next().ok_or(())?.parse().map_err(drop)?,
            server_id: iter.next().ok_or(())?.parse().map_err(drop)?,
            motd2: iter.next().ok_or(())?.to_string(),
            gamemode: iter.next().ok_or(())?.to_string(),
            gamemode_num: iter.next().ok_or(())?.parse().map_err(drop)?,
            port: iter.next().ok_or(())?.parse().map_err(drop)?,
            port6: iter.next().ok_or(())?.parse().map_err(drop)?,
        })
    }
}

#[derive(Serialize)]
pub struct BedrockRet {
    resp: BedrockParsed,
    ping: f64,
}

#[get("/bedrock?<hostname>&<port>")]
pub async fn bedrock(
    resolver: &State<TokioAsyncResolver>,
    hostname: &str,
    port: Option<u16>,
) -> Result<Json<BedrockRet>, ()> {
    let addr = resolver
        .lookup_ip(hostname)
        .await
        .ok()
        .and_then(|r| r.iter().next());
    let sockaddr = match addr {
        Some(std::net::IpAddr::V4(v4)) => (v4, port.unwrap_or(19132))
            .to_socket_addrs()
            .map_err(drop)?
            .next(),
        Some(std::net::IpAddr::V6(v6)) => (v6, port.unwrap_or(19133))
            .to_socket_addrs()
            .map_err(drop)?
            .next(),
        None => (hostname, port.unwrap_or(19132))
            .to_socket_addrs()
            .map_err(drop)?
            .next(),
    }
    .ok_or(())?;

    let (latency, data) = rust_raknet::RaknetSocket::ping(&sockaddr)
        .await
        .map_err(drop)?;

    Ok(BedrockRet {
        resp: BedrockParsed::from_str(&data)?,
        ping: Duration::from_millis(latency as u64).as_secs_f64(),
    }
    .into())
}
