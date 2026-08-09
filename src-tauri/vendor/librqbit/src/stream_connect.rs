use std::net::SocketAddr;

use anyhow::Context;

#[derive(Debug, Clone)]
pub(crate) struct SocksProxyConfig {
    pub host: String,
    pub port: u16,
    pub username_password: Option<(String, String)>,
}

impl SocksProxyConfig {
    pub fn parse(url: &str) -> anyhow::Result<Self> {
        let url = ::url::Url::parse(url).context("invalid proxy URL")?;
        if url.scheme() != "socks5" {
            anyhow::bail!("proxy URL should have socks5 scheme");
        }
        let host = url.host_str().context("missing host")?;
        let port = url.port().context("missing port")?;
        let up = url
            .password()
            .map(|p| (url.username().to_owned(), p.to_owned()));
        Ok(Self {
            host: host.to_owned(),
            port,
            username_password: up,
        })
    }

    async fn connect(
        &self,
        addr: SocketAddr,
    ) -> anyhow::Result<(
        impl tokio::io::AsyncRead + Unpin,
        impl tokio::io::AsyncWrite + Unpin,
    )> {
        let proxy_addr = (self.host.as_str(), self.port);

        let stream = if let Some((username, password)) = self.username_password.as_ref() {
            tokio_socks::tcp::Socks5Stream::connect_with_password(
                proxy_addr,
                addr,
                username.as_str(),
                password.as_str(),
            )
            .await
            .context("error connecting to proxy")?
        } else {
            tokio_socks::tcp::Socks5Stream::connect(proxy_addr, addr)
                .await
                .context("error connecting to proxy")?
        };

        Ok(tokio::io::split(stream))
    }
}

#[derive(Debug, Default)]
pub(crate) struct StreamConnector {
    proxy_config: Option<SocksProxyConfig>,
    /// LOCAL PATCH: source address for outbound peer connections.
    bind_ip: Option<std::net::IpAddr>,
}

impl From<Option<SocksProxyConfig>> for StreamConnector {
    fn from(proxy_config: Option<SocksProxyConfig>) -> Self {
        Self {
            proxy_config,
            bind_ip: None,
        }
    }
}

impl StreamConnector {
    /// LOCAL PATCH: construct with an optional outbound source address.
    pub fn new(
        proxy_config: Option<SocksProxyConfig>,
        bind_ip: Option<std::net::IpAddr>,
    ) -> Self {
        Self {
            proxy_config,
            bind_ip,
        }
    }

    /// LOCAL PATCH: plain `TcpStream::connect` lets the OS pick the source
    /// interface via the routing table, which defeats interface binding. Build
    /// the socket manually so the source address is bound before connecting.
    async fn connect_bound(
        bind_ip: std::net::IpAddr,
        addr: SocketAddr,
    ) -> anyhow::Result<tokio::net::TcpStream> {
        // An IPv4 source can't reach an IPv6 destination or vice versa. Fail
        // loudly rather than silently falling back to an unbound socket.
        if bind_ip.is_ipv4() != addr.is_ipv4() {
            anyhow::bail!("cannot connect to {addr} from {bind_ip}: address family mismatch");
        }

        let socket = if addr.is_ipv4() {
            tokio::net::TcpSocket::new_v4()
        } else {
            tokio::net::TcpSocket::new_v6()
        }
        .context("error creating outbound socket")?;

        socket
            .bind(SocketAddr::new(bind_ip, 0))
            .with_context(|| format!("error binding outbound socket to {bind_ip}"))?;

        socket
            .connect(addr)
            .await
            .with_context(|| format!("error connecting to {addr} from {bind_ip}"))
    }

    pub async fn connect(
        &self,
        addr: SocketAddr,
    ) -> anyhow::Result<(
        Box<dyn tokio::io::AsyncRead + Send + Unpin>,
        Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    )> {
        if let Some(proxy) = self.proxy_config.as_ref() {
            let (r, w) = proxy.connect(addr).await?;
            return Ok((Box::new(r), Box::new(w)));
        }

        let stream = match self.bind_ip {
            Some(ip) => Self::connect_bound(ip, addr).await?,
            None => tokio::net::TcpStream::connect(addr)
                .await
                .context("error connecting")?,
        };
        let (r, w) = stream.into_split();
        Ok((Box::new(r), Box::new(w)))
    }
}
