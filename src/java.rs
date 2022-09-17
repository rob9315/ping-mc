use miners::encoding::decode;
use miners::net::{conn::ReadHalf, encoding::Encoder};
use miners::protocol::netty::{status::clientbound::Response0, CbStatus};
use miners::protocol::ToStatic;

#[derive(Serialize)]
pub struct JavaResult<'a> {
    resp: std::borrow::Cow<'a, str>,
    ping: Option<f64>,
}

#[get("/java?<hostname>&<port>&<version>")]
pub async fn java<'a>(
    resolver: &State<TokioAsyncResolver>,
    hostname: &str,
    port: Option<u16>,
    version: Option<u32>,
) -> Result<Json<JavaResult<'a>>, ()> {
    let port = port.unwrap_or(25565);

    let (ip, port) = match resolver
        .srv_lookup(format!("_minecraft._tcp.{hostname}"))
        .await
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

    let statusrequest = miners::protocol::netty::status::serverbound::Request0 {};

    let statusrequestencoded = encoder.encode(0, statusrequest).map_err(drop)?;

    w.write(statusrequestencoded).await.map_err(drop)?;
    w.flush().await.map_err(drop)?;

    let recv_loop = read_status_response_loop(&mut r);
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), recv_loop)
        .await
        .map_err(drop)?
        .map_err(drop)?
        .into_static();

    let pingrequest = miners::protocol::netty::status::serverbound::Ping0 { time: 0 };
    let pingrequestencoded = encoder.encode(1, pingrequest).map_err(drop)?;
    w.write(pingrequestencoded).await.map_err(drop)?;
    w.flush().await.map_err(drop)?;

    let req_time = std::time::SystemTime::now();
    let ping_resp = r.read_encoded().await.map_err(drop)?;

    let ping = std::time::SystemTime::now()
        .duration_since(req_time)
        .unwrap_or_default();
    let ping = if let Ok((id, data)) = ping_resp.into_packet() {
        status_cb(id, data).ok().map(|_| ping.as_secs_f64())
    } else {
        None
    };

    Ok(JavaResult {
        resp: resp.data,
        ping,
    }
    .into())
}
#[allow(clippy::needless_lifetimes)]
async fn read_status_response_loop<'r, R: AsyncRead + Unpin>(
    r: &'r mut ReadHalf<R>,
) -> decode::Result<Response0<'r>> {
    loop {
        let thing = read_status_response(r).await?;
        // SAFETY: in dire need of polonius (yes it's safe)
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
use rocket::futures::AsyncRead;
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use trust_dns_resolver::TokioAsyncResolver;
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
