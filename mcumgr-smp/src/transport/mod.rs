use core::ops::DerefMut;

mod error;
pub use error::{Error, Result};
use tokio::io::{self};

#[cfg(feature = "transport-serial")]
pub mod serial;

#[cfg(feature = "payload-cbor")]
mod cbor;
#[cfg(feature = "payload-cbor")]
pub use cbor::CBORTransporter;

/// Async Transport layer trait.
///
/// This does not do framing like serial requires.
pub trait SMPTransport: io::AsyncRead + io::AsyncWrite {}

impl<P> SMPTransport for core::pin::Pin<P>
where
    P: DerefMut + Unpin,
    P::Target: SMPTransport + Unpin,
{
}

macro_rules! deref_async_transport {
    () => {};
}

impl<T: ?Sized + SMPTransport + Unpin> SMPTransport for Box<T> {
    deref_async_transport!();
}

impl<T: ?Sized + SMPTransport + Unpin> SMPTransport for &mut T {
    deref_async_transport!();
}
