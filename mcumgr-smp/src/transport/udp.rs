use super::{Result, SMPTransport};

use tokio::io;

pub struct UDPTransport(tokio::net::UdpSocket);

impl UDPTransport {
    pub async fn new<A: tokio::net::ToSocketAddrs>(target: A) -> Result<Self> {
        let socket = tokio::net::UdpSocket::bind((std::net::Ipv6Addr::UNSPECIFIED, 0)).await?;
        socket.connect(target).await?;

        Ok(Self(socket))
    }
}

impl io::AsyncRead for UDPTransport {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.0.poll_recv(cx, buf)
    }
}

impl io::AsyncWrite for UDPTransport {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::prelude::v1::Result<usize, std::io::Error>> {
        self.0.poll_send(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::prelude::v1::Result<(), std::io::Error>> {
        std::task::Poll::Ready(Err(std::io::ErrorKind::Unsupported.into()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::prelude::v1::Result<(), std::io::Error>> {
        std::task::Poll::Ready(Err(std::io::ErrorKind::Unsupported.into()))
    }
}

impl SMPTransport for UDPTransport {}
