#[allow(unused)]
use std::sync::atomic::AtomicUsize;
use std::{net::ToSocketAddrs, str::FromStr, time::Duration};
use rocket_cors::{AllowedOrigins, CorsOptions};

use miners::{
    encoding::decode,
    net::{conn::ReadHalf, encoding::Encoder},
    protocol::{
        netty::{status::clientbound::Response0, CbStatus},
        ToStatic,
    },
};
use rocket::{futures::AsyncRead, serde::json::Json, State};
use serde::Serialize;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    TokioAsyncResolver, TokioHandle,
};

#[macro_use]
extern crate rocket;

#[derive(Serialize)]
struct JavaResult<'a> {
    resp: std::borrow::Cow<'a, str>,
    ping: Option<f64>,
}

#[get("/java?<hostname>&<port>&<version>")]
async fn java<'a>(
    resolver: &State<TokioAsyncResolver>,
    hostname: &str,
    port: Option<u16>,
    version: Option<u32>,
) -> Result<Json<JavaResult<'a>>, ()> {
    // REQ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let port = port.unwrap_or(25565);

    // eprintln!("req: {hostname}:{port}");

    let (ip, port) = match resolver
        .srv_lookup(format!("_minecraft._tcp.{hostname}"))
        .await
        // .map_err(|e| {
        //     eprintln!("{e:?}");
        // })
        .ok()
        .and_then(|a| a.iter().next().map(|x| (x.target().clone(), x.port())))
    {
        Some((target, port)) => {
            // eprintln!("srv");
            (resolver.lookup_ip(target).await, port)
        }
        None => (resolver.lookup_ip(hostname).await, port),
    };
    let ip = ip.map_err(drop)?.iter().next().ok_or(())?;
    // eprintln!("ip: {ip}, port: {port}");

    let mut stream = tokio::net::TcpStream::connect((ip, port))
        .await
        .map_err(drop)?;

    let (r, w) = stream.split();

    let (mut r, mut w) = miners::net::conn::Connection::new(r.compat(), w.compat_write()).split();

    let mut encoder = Encoder::default();

    let handshake = miners::protocol::netty::handshaking::serverbound::Handshake0 {
        protocol_version: version.unwrap_or(0) as i32,
        server_address: hostname.into(),
        server_port: port,
        next_state: miners::protocol::netty::handshaking::serverbound::NextState0::Status,
    };

    let handshakeencoded = encoder.encode(0, handshake).map_err(drop)?;

    w.write(handshakeencoded).await.map_err(drop)?;

    // eprintln!("handshake written");

    let statusrequest = miners::protocol::netty::status::serverbound::Request0 {};

    let statusrequestencoded = encoder.encode(0, statusrequest).map_err(drop)?;

    w.write(statusrequestencoded).await.map_err(drop)?;
    w.flush().await.map_err(drop)?;

    // eprintln!("statusrequest written");

    let recv_loop = read_status_response_loop(&mut r);
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), recv_loop)
        .await
        .map_err(drop)?
        .map_err(drop)?
        .into_static();

    // eprintln!("statusresponse received");

    let pingrequest = miners::protocol::netty::status::serverbound::Ping0 { time: 0 };
    let pingrequestencoded = encoder.encode(1, pingrequest).map_err(drop)?;
    w.write(pingrequestencoded).await.map_err(drop)?;
    w.flush().await.map_err(drop)?;

    // eprintln!("pingrequest written");

    let req_time = std::time::SystemTime::now();
    let ping_resp = r.read_encoded().await.map_err(drop)?;

    // eprintln!("pingresponse received");

    let ping = std::time::SystemTime::now()
        .duration_since(req_time)
        .unwrap_or_default();
    let ping = if let Ok((id, data)) = ping_resp.into_packet() {
        // eprintln!("ping: {ping:?}");

        status_cb(id, data).ok().map(|_| ping.as_secs_f64())
    } else {
        None
    };

    // REQ_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

    Ok(JavaResult {
        resp: resp.data,
        ping,
    }
    .into())
}

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
struct BedrockRet {
    resp: BedrockParsed,
    ping: f64,
}

#[get("/bedrock?<hostname>&<port>")]
async fn bedrock(
    resolver: &State<TokioAsyncResolver>,
    hostname: &str,
    port: Option<u16>,
) -> Result<Json<BedrockRet>, ()> {

    // eprintln!("req: {hostname}:{port:?}");
    
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

    // eprintln!("resolved: {sockaddr}");

    let (latency, data) = rust_raknet::RaknetSocket::ping(&sockaddr)
        .await
        .map_err(drop)?;

    // eprintln!("data: {data}");
    // eprintln!("latency: {latency}");

    Ok(BedrockRet {
        resp: BedrockParsed::from_str(&data)?,
        ping: Duration::from_millis(latency as u64).as_secs_f64(),
    }
    .into())
}

// static REQ_COUNT: AtomicUsize = AtomicUsize::new(0);

#[launch]
fn rocket() -> _ {
    // tokio::spawn(async {
    //     loop {
    //         println!(
    //             "reqs: {:<10}",
    //             REQ_COUNT.load(std::sync::atomic::Ordering::Relaxed)
    //         );
    //         tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    //     }
    // });
    let cors = CorsOptions::default()
        .allowed_origins(AllowedOrigins::all())
        .allowed_methods(
            vec![Method::Get]
                .into_iter()
                .map(From::from)
                .collect(),
            )
        .allow_credentials(false)
        .to_cors();

    let resolver = TokioAsyncResolver::tokio_from_system_conf().unwrap_or_else(|_| {
        TokioAsyncResolver::new(
            ResolverConfig::default(),
            ResolverOpts::default(),
            TokioHandle,
        )
        .expect("couldn't construct dns resolver")
    });
    rocket::build()
        .attach(cors.unwrap())
        .manage(resolver)
        .mount("/", routes![java, bedrock])
}

#[allow(clippy::needless_lifetimes)]
async fn read_status_response_loop<'r, R: AsyncRead + Unpin>(
    r: &'r mut ReadHalf<R>,
) -> decode::Result<Response0<'r>> {
    loop {
        let thing = read_status_response(r).await?;
        if let Some(resp) = unsafe { std::mem::transmute(thing) } {
            return Ok(resp);
        };
    }
}
const JAVA_PV: u32 = 0;

async fn read_status_response<R: AsyncRead + Unpin>(
    r: &mut ReadHalf<R>,
) -> decode::Result<Option<Response0>> {
    let x = r.read_encoded().await?;
    let (id, data) = x.into_packet()?;

    let status = status_cb(id, data)?;

    if let CbStatus::Response0(resp) = status {
        return Ok(Some(resp));
    }
    Ok(None)
}
use miners::encoding::Decode;
fn status_cb(id: i32, data: &[u8]) -> decode::Result<CbStatus> {
    let mut rd = std::io::Cursor::new(data);

    miners::protocol::status_cb_tree! {
        id, JAVA_PV, {
            Ok(CbStatus::#PacketName(#PacketType::decode(&mut rd)?))
        },
        {
            Err(decode::Error::InvalidId)
        }
    }
}
