use std::future::Future;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tracing::*;

use super::{Error, Result, SMPTransport};
use crate::SMPFrame;

pub struct CBORTransporter<T: tokio::io::AsyncRead = Box<dyn SMPTransport + Unpin>> {
    transport: tokio::io::BufReader<T>,
    #[cfg(feature = "serial")]
    serial_framing: bool,
}

impl<T: tokio::io::AsyncRead> CBORTransporter<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport: tokio::io::BufReader::new(transport),
            #[cfg(feature = "serial")]
            serial_framing: false,
        }
    }

    #[cfg(feature = "serial")]
    pub fn new_serial(transport: T) -> Self {
        Self {
            transport: tokio::io::BufReader::new(transport),
            serial_framing: true,
        }
    }

    /// Gets a reference to the underlying transport.
    pub fn get_ref(&self) -> &T {
        self.transport.get_ref()
    }

    /// Gets a mutable reference to the underlying transport.
    pub fn get_mut(&mut self) -> &mut T {
        self.transport.get_mut()
    }
}

impl<T: SMPTransport + Unpin> CBORTransporter<T> {
    pub fn send_frame<'a>(
        &'a mut self,
        frame: &'a [u8],
    ) -> impl Future<Output = std::io::Result<usize>> + 'a {
        self.transport.write(frame)
    }

    pub fn send_frame_all<'a>(
        &'a mut self,
        frame: &'a [u8],
    ) -> impl Future<Output = std::io::Result<()>> + 'a {
        self.transport.write_all(frame)
    }

    pub fn recv_frame<'a>(
        &'a mut self,
        frame: &'a mut [u8],
    ) -> impl Future<Output = std::io::Result<usize>> + 'a {
        self.transport.read(frame)
    }

    /// Sends a CBOR frame
    pub async fn send<F: serde::Serialize>(&mut self, frame: SMPFrame<F>) -> Result {
        let bytes = frame.encode_with_cbor();
        #[cfg(feature = "serial")]
        if self.serial_framing {
            return self.send_serial(bytes).await;
        }
        self.send_frame_all(&bytes).await.map_err(Error::from)
    }

    /// Receive a CBOR frame
    pub async fn recv<F: serde::de::DeserializeOwned>(&mut self) -> Result<SMPFrame<F>> {
        #[cfg(feature = "serial")]
        let buf = if self.serial_framing {
            self.recv_serial().await?
        } else {
            self.recv_full_frame().await?
        };
        #[cfg(not(feature = "serial"))]
        let buf = self.recv_full_frame().await?;

        SMPFrame::<F>::decode_with_cbor(&buf).map_err(Error::from)
    }

    pub async fn transceive<O, I>(&mut self, frame: SMPFrame<O>) -> Result<SMPFrame<I>>
    where
        O: serde::Serialize,
        I: serde::de::DeserializeOwned,
    {
        self.send(frame).await?;
        self.recv().await
    }

    /// Receive a full not framed frame. Length is decoded from byte 2 and 3 in the inital bytes.
    async fn recv_full_frame(&mut self) -> Result<Vec<u8>> {
        let mut ret = Vec::new();
        let mut buf = [0u8; 4096];

        let read = self.transport.read(&mut buf).await?;
        ret.extend(&buf[..read]);

        let len = u16::from_be_bytes([ret[2], ret[3]]);
        debug!("Received {}/{} bytes", ret.len(), len);
        while ret.len() < (len + 8) as usize {
            trace!("Waiting on next chunk");
            let recv = self.transport.read(&mut buf).await?;
            if recv == 0 {
                return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            ret.extend(&buf[..recv]);
            debug!("Received {}/{} bytes", ret.len(), len);
        }
        Ok(ret)
    }
}

#[cfg(feature = "serial")]
impl<T: SMPTransport + Unpin> CBORTransporter<T> {
    async fn send_serial(&mut self, bytes: Vec<u8>) -> Result {
        let mut buf = Vec::with_capacity(128);
        buf.resize(128, 0);

        let mut encoder = crate::smp_framing::SMPTransportEncoder::new(&bytes);

        while !encoder.is_complete() {
            let len = encoder.write_line(&mut buf)?;
            self.send_frame_all(&buf[..len]).await?;
        }

        Ok(())
    }

    async fn recv_serial(&mut self) -> Result<Vec<u8>> {
        let mut decoder = crate::smp_framing::SMPTransportDecoder::new();

        let mut buf = Vec::new();

        while !decoder.is_complete() {
            let len = self.transport.read_until(0xa, &mut buf).await?;
            decoder.input_line(&buf[..len])?;
        }

        decoder.into_frame_payload().map_err(Error::from)
    }
}

#[cfg(test)]
mod test {
    use tokio::io;

    use super::*;

    struct VecTransport {
        write: Vec<u8>,
        chunk_size: usize,
        read: Vec<u8>,
    }

    impl VecTransport {
        pub fn new() -> Self {
            Self {
                write: Vec::new(),
                chunk_size: usize::MAX,
                read: Vec::new(),
            }
        }
    }

    impl io::AsyncRead for VecTransport {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let me = self.get_mut();
            let buf_inner = unsafe { buf.inner_mut() };
            let len = std::cmp::min(buf_inner.len(), me.read.len());
            let len = std::cmp::min(len, me.chunk_size);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    me.read.as_ptr(),
                    buf_inner.as_mut_ptr().cast(),
                    len,
                );
                buf.assume_init(len);
            };
            buf.set_filled(len);
            me.read.drain(..len);
            core::task::Poll::Ready(Ok(()))
        }
    }

    impl io::AsyncWrite for VecTransport {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::prelude::v1::Result<usize, std::io::Error>> {
            let len = std::cmp::min(buf.len(), self.chunk_size);
            self.get_mut().write.extend_from_slice(&buf[..len]);
            core::task::Poll::Ready(Ok(len))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::prelude::v1::Result<(), std::io::Error>> {
            todo!()
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::prelude::v1::Result<(), std::io::Error>> {
            todo!()
        }
    }

    impl SMPTransport for VecTransport {}

    #[tokio::test]
    async fn vec_transport_read() -> Result {
        let mut buf = Vec::new();
        buf.resize(5, 0);
        let transport = VecTransport::new();
        let mut cbor = CBORTransporter::new(transport);

        let read = cbor.recv_frame(&mut buf).await?;
        assert_eq!(read, 0);

        cbor.get_mut()
            .read
            .extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        let read = cbor.recv_frame(&mut buf).await?;
        assert_eq!(read, 4);
        assert_eq!(&buf[..read], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(cbor.get_ref().read.len(), 0);

        cbor.get_mut()
            .read
            .extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        cbor.get_mut().chunk_size = 2;

        let read = cbor.recv_frame(&mut buf).await?;
        assert_eq!(read, 2);
        assert_eq!(&buf[..2], &[0x01, 0x02]);
        let read = cbor.recv_frame(&mut buf).await?;
        assert_eq!(read, 2);
        assert_eq!(&buf[..2], &[0x03, 0x04]);

        Ok(())
    }

    #[tokio::test]
    async fn vec_transport_write() -> Result {
        let mut cbor = CBORTransporter::new(VecTransport::new());

        let write = cbor.send_frame(&[0x01, 0x02]).await?;
        assert_eq!(write, 2);
        assert_eq!(&cbor.get_ref().write, &[0x01, 0x02]);
        cbor.get_mut().write.clear();

        let buf = &[0x01, 0x02, 0x03, 0x04];

        cbor.get_mut().chunk_size = 2;
        let write = cbor.send_frame(buf).await?;
        assert_eq!(write, 2);
        assert_eq!(&cbor.get_ref().write, &[0x01, 0x02]);
        cbor.get_mut().write.clear();

        cbor.send_frame_all(buf).await?;
        assert_eq!(&cbor.get_ref().write, buf);

        Ok(())
    }

    #[tokio::test]
    async fn read_frame() -> Result {
        let bytes = &[
            0x0, 0x0, 0x0, 0x8, 0x0, 0x0, 0x7, 0x0, 0xa1, 0x65, 0x66, 0x6f, 0x72, 0x63, 0x65, 0x0,
        ];
        let mut cbor = CBORTransporter::new(VecTransport::new());

        cbor.get_mut().read.extend(bytes);

        let frame = cbor.recv::<crate::os_management::ResetRequest>().await;
        assert!(frame.is_ok());
        assert_eq!(cbor.get_ref().read.len(), 0);

        // chunk_size = 4
        cbor.get_mut().chunk_size = 4;
        cbor.get_mut().read.extend(bytes);

        let frame = cbor.recv::<crate::os_management::ResetRequest>().await;
        assert!(frame.is_ok());
        assert_eq!(cbor.get_ref().read.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn write_frame() -> Result {
        let mut cbor = CBORTransporter::new(VecTransport::new());

        cbor.send(crate::smp::SMPFrame::new(
            crate::OpCode::ReadRequest,
            7,
            crate::Group::Default,
            0,
            crate::os_management::ResetRequest { force: 0 },
        ))
        .await?;
        assert_eq!(
            &cbor.get_ref().write,
            &[
                0x0, 0x0, 0x0, 0x8, 0x0, 0x0, 0x7, 0x0, 0xa1, 0x65, 0x66, 0x6f, 0x72, 0x63, 0x65,
                0x0
            ]
        );
        let frame = crate::smp::SMPFrame::<crate::os_management::ResetRequest>::decode_with_cbor(
            &cbor.get_ref().write,
        );
        assert!(frame.is_ok());

        // chunk_size = 4
        cbor.get_mut().write.clear();
        cbor.get_mut().chunk_size = 4;

        cbor.send(crate::smp::SMPFrame::new(
            crate::OpCode::ReadRequest,
            7,
            crate::Group::Default,
            0,
            crate::os_management::ResetRequest { force: 0 },
        ))
        .await?;
        assert_eq!(
            &cbor.get_ref().write,
            &[
                0x0, 0x0, 0x0, 0x8, 0x0, 0x0, 0x7, 0x0, 0xa1, 0x65, 0x66, 0x6f, 0x72, 0x63, 0x65,
                0x0
            ]
        );
        let frame = crate::smp::SMPFrame::<crate::os_management::ResetRequest>::decode_with_cbor(
            &cbor.get_ref().write,
        );
        assert!(frame.is_ok());

        Ok(())
    }

    #[cfg(feature = "serial")]
    #[tokio::test]
    async fn read_serial_frame() -> Result {
        let bytes = &[
            6, 9, 65, 66, 73, 65, 65, 65, 65, 73, 65, 65, 65, 72, 65, 75, 70, 108, 90, 109, 57,
            121, 89, 50, 85, 65, 75, 119, 73, 61, 10,
        ];
        let mut cbor = CBORTransporter::new_serial(VecTransport::new());

        cbor.get_mut().read.extend(bytes);

        let frame = cbor.recv::<crate::os_management::ResetRequest>().await;
        assert!(frame.is_ok());
        assert_eq!(cbor.get_ref().read.len(), 0);

        Ok(())
    }

    #[cfg(feature = "serial")]
    #[tokio::test]
    async fn write_serial_frame() -> Result {
        let mut cbor = CBORTransporter::new_serial(VecTransport::new());

        cbor.send(crate::smp::SMPFrame::new(
            crate::OpCode::ReadRequest,
            7,
            crate::Group::Default,
            0,
            crate::os_management::ResetRequest { force: 0 },
        ))
        .await?;
        assert_eq!(
            &cbor.get_ref().write,
            &[
                6, 9, 65, 66, 73, 65, 65, 65, 65, 73, 65, 65, 65, 72, 65, 75, 70, 108, 90, 109, 57,
                121, 89, 50, 85, 65, 75, 119, 73, 61, 10
            ]
        );

        Ok(())
    }
}
