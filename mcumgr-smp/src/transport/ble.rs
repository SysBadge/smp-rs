use btleplug::api::Peripheral;
use futures::StreamExt;

use tokio::io;

use super::{Result, SMPTransport};

const MAX_MTU: usize = 252;
pub const BLE_TRANSPORT_CHARACTERISTIC_UUID: uuid::Uuid =
    uuid::uuid!("DA2E7828-FBCE-4E01-AE9E-261174997C48");
pub const BLE_TRANSPORT_SERVICE_UUID: uuid::Uuid = uuid::uuid!("8D53DC1D1DB74CD3868B8A527460AA84");

enum SendMessage {
    Close,
    Send((usize, [u8; MAX_MTU])),
}

pub struct BLETransport {
    recv: tokio::sync::mpsc::Receiver<Vec<u8>>,
    send: tokio_util::sync::PollSender<SendMessage>,
    peripheral: btleplug::platform::Peripheral,
    recv_task: tokio::task::JoinHandle<()>,
    send_task: tokio::task::JoinHandle<()>,
}

impl BLETransport {
    pub async fn new(peripheral: btleplug::platform::Peripheral) -> Result<Self> {
        if !peripheral.is_connected().await? {
            peripheral.connect().await?;
        }
        peripheral.discover_services().await?;
        let characteristic = peripheral
            .characteristics()
            .iter()
            .find(|c| c.uuid == BLE_TRANSPORT_CHARACTERISTIC_UUID)
            .unwrap()
            .clone();

        peripheral.subscribe(&characteristic).await?;
        let notifications = peripheral.notifications().await?;

        let (recv_send, recv_recv) = tokio::sync::mpsc::channel(10);
        let (send_send, send_recv) = tokio::sync::mpsc::channel(1);
        let send_send = tokio_util::sync::PollSender::new(send_send);

        let recv_task = tokio::task::spawn(async move {
            let mut notifications = notifications;
            let recv_send = recv_send;
            loop {
                let notification = notifications.next().await.expect("??");
                recv_send.send(notification.value).await.expect("??");
            }
        });
        let p = peripheral.clone();
        let send_task = tokio::task::spawn(async move {
            let peripheral = p;
            let mut channel = send_recv;
            loop {
                let task = channel.recv().await;
                match task {
                    Some(SendMessage::Send((len, buf))) => {
                        if let Err(_e) = peripheral
                            .write(
                                &characteristic,
                                &buf[..len],
                                btleplug::api::WriteType::WithoutResponse,
                            )
                            .await
                        {
                            channel.close();
                        }
                    }
                    None | Some(SendMessage::Close) => {
                        let _ = peripheral.disconnect().await;
                        return;
                    }
                }
            }
        });

        Ok(Self {
            recv: recv_recv,
            send: send_send,
            peripheral,
            recv_task,
            send_task,
        })
    }

    pub async fn find_name(name: &str) -> Result<Self> {
        use btleplug::api::{Central, Manager};
        let manager = btleplug::platform::Manager::new().await?;
        let adapters = manager.adapters().await?;
        let central = adapters
            .into_iter()
            .nth(0)
            .ok_or(super::Error::Io(std::io::ErrorKind::NotFound.into()))?;

        central
            .start_scan(btleplug::api::ScanFilter {
                services: vec![BLE_TRANSPORT_SERVICE_UUID],
            })
            .await?;

        let peripheral = find_mcumgr(&central, name).await.ok_or(super::Error::Io(
            std::io::ErrorKind::AddrNotAvailable.into(),
        ))?;

        Self::new(peripheral).await
    }
}

impl io::AsyncRead for BLETransport {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let recv = std::task::ready!(self.get_mut().recv.poll_recv(cx))
            .ok_or(std::io::ErrorKind::ConnectionAborted)?;
        let buf_inner = unsafe { buf.inner_mut() };
        let len = std::cmp::min(buf_inner.len(), recv.len());
        unsafe {
            core::ptr::copy_nonoverlapping(recv.as_ptr(), buf_inner.as_mut_ptr().cast(), len);
            buf.assume_init(len);
        };
        buf.set_filled(len);
        core::task::Poll::Ready(Ok(()))
    }
}

impl io::AsyncWrite for BLETransport {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::prelude::v1::Result<usize, std::io::Error>> {
        let len = std::cmp::min(buf.len(), MAX_MTU);
        let mut notify = (len, [0u8; MAX_MTU]);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), notify.1.as_mut_ptr(), notify.0);
        }
        let me = self.get_mut();
        std::task::ready!(me.send.poll_reserve(cx))
            .map_err(|_| std::io::ErrorKind::ConnectionAborted)?;
        me.send
            .send_item(SendMessage::Send(notify))
            .map_err(|_| std::io::ErrorKind::ConnectionAborted)?;
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

impl SMPTransport for BLETransport {}

impl Drop for BLETransport {
    fn drop(&mut self) {
        while self
            .send
            .get_ref()
            .unwrap()
            .try_send(SendMessage::Close)
            .is_err()
        {}
        self.recv_task.abort();
        while self.send_task.is_finished() {}
    }
}

async fn find_mcumgr(
    central: &btleplug::platform::Adapter,
    name: &str,
) -> Option<btleplug::platform::Peripheral> {
    use btleplug::api::{Central, Peripheral};
    for _ in 0..10 {
        for p in central.peripherals().await.unwrap() {
            tracing::trace!("{:?}", p.properties().await);
            if p.properties()
                .await
                .unwrap()
                .unwrap()
                .local_name
                .iter()
                .any(|p_name| p_name.contains(name))
            {
                return Some(p);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    None
}
