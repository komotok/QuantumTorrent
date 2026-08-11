use std::io::SeekFrom;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use futures_util::TryStreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use librqbit::ManagedTorrent;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;

const READ_BUF: usize = 64 * 1024;

pub type Resolver = Arc<dyn Fn(usize) -> Option<Arc<ManagedTorrent>> + Send + Sync>;

pub struct PreviewServer {
    pub port: u16,
    pub token: String,
}

type Body = BoxBody<Bytes, std::io::Error>;

fn empty(status: StatusCode) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::new()).map_err(|e| match e {}).boxed());
    *r.status_mut() = status;
    r
}

/// `bytes=N-` and `bytes=N-M`. Anything else is ignored and the whole file is
/// served, which is what a player expects from a 200.
fn parse_range(raw: &str) -> Option<(u64, Option<u64>)> {
    let spec = raw.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end = end.trim();
    if end.is_empty() {
        Some((start, None))
    } else {
        let end: u64 = end.parse().ok()?;
        if end < start {
            return None;
        }
        Some((start, Some(end)))
    }
}

async fn handle(
    req: Request<hyper::body::Incoming>,
    resolver: Resolver,
    token: Arc<str>,
) -> Result<Response<Body>, std::io::Error> {
    if !matches!(*req.method(), hyper::Method::GET | hyper::Method::HEAD) {
        return Ok(empty(StatusCode::METHOD_NOT_ALLOWED));
    }

    // /{token}/{torrent_id}/{file_index}/{filename}
    let path = req.uri().path().trim_start_matches('/');
    let mut parts = path.splitn(4, '/');
    let (Some(got_token), Some(id), Some(file_index)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Ok(empty(StatusCode::NOT_FOUND));
    };

    if got_token != &*token {
        return Ok(empty(StatusCode::FORBIDDEN));
    }

    let (Ok(id), Ok(file_index)) = (id.parse::<usize>(), file_index.parse::<usize>()) else {
        return Ok(empty(StatusCode::NOT_FOUND));
    };

    let Some(handle) = resolver(id) else {
        return Ok(empty(StatusCode::NOT_FOUND));
    };

    let Ok(mut stream) = handle.stream(file_index) else {
        return Ok(empty(StatusCode::NOT_FOUND));
    };

    let total = stream.len();
    let range = req
        .headers()
        .get(hyper::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range);

    let mime = mime_guess::from_path(path).first_raw().unwrap_or("application/octet-stream");

    let mut builder = Response::builder()
        .header(hyper::header::ACCEPT_RANGES, "bytes")
        .header(hyper::header::CONTENT_TYPE, mime);

    let (status, length) = match range {
        Some((start, end)) if start < total => {
            let end = end.unwrap_or(total - 1).min(total - 1);
            stream.seek(SeekFrom::Start(start)).await?;
            builder = builder.header(
                hyper::header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{total}"),
            );
            (StatusCode::PARTIAL_CONTENT, end - start + 1)
        }
        Some(_) => {
            // Unsatisfiable: start past EOF.
            let mut r = empty(StatusCode::RANGE_NOT_SATISFIABLE);
            r.headers_mut().insert(
                hyper::header::CONTENT_RANGE,
                format!("bytes */{total}").parse().unwrap(),
            );
            return Ok(r);
        }
        None => (StatusCode::OK, total),
    };

    builder = builder.status(status).header(hyper::header::CONTENT_LENGTH, length);

    if req.method() == hyper::Method::HEAD {
        return Ok(builder
            .body(Full::new(Bytes::new()).map_err(|e| match e {}).boxed())
            .unwrap());
    }

    let reader = stream.take(length);
    let body = StreamBody::new(
        ReaderStream::with_capacity(reader, READ_BUF).map_ok(Frame::data),
    );
    Ok(builder.body(body.boxed()).unwrap())
}

pub async fn start(resolver: Resolver) -> std::io::Result<PreviewServer> {
    let token: String = {
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.gen_range(0..36)]))
            .collect()
    };

    // Loopback only. The token keeps other local processes from enumerating
    // torrents through this, since anything on the machine can reach 127.0.0.1.
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let port = listener.local_addr()?.port();

    let shared_token: Arc<str> = token.clone().into();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            let resolver = resolver.clone();
            let token = shared_token.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req| handle(req, resolver.clone(), token.clone()));
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(sock), service)
                    .await;
            });
        }
    });

    Ok(PreviewServer { port, token })
}
