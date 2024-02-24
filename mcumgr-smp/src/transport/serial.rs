//! Serial Transport layer implementation

use core::{
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io;
use tokio_serial::SerialPortBuilderExt;

use super::{Result, SMPTransport};

pub struct SerialTransport<P = tokio_serial::SerialStream> {
    port: P,
}

impl SerialTransport {
    pub fn open<'a>(path: impl Into<std::borrow::Cow<'a, str>>, baud_rate: u32) -> Result<Self> {
        let port = tokio_serial::new(path, baud_rate).open_native_async()?;

        Ok(Self { port })
    }
}

impl<P: io::AsyncRead + Unpin> io::AsyncRead for SerialTransport<P> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().port).poll_read(cx, buf)
    }
}

impl<P: io::AsyncWrite + Unpin> io::AsyncWrite for SerialTransport<P> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::prelude::v1::Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().port).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::prelude::v1::Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().port).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::prelude::v1::Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().port).poll_shutdown(cx)
    }
}

impl<P: io::AsyncRead + io::AsyncWrite + Unpin> SMPTransport for SerialTransport<P> {}
