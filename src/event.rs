use actix::Message;
use actix_web::Binary;
use futures::{Future, Poll};
use libc::c_ushort;
use tokio_pty_process::PtyMaster;

pub use crate::terminado::TerminadoMessage;

use tokio_codec::{BytesCodec, Decoder};
type BytesMut = <BytesCodec as Decoder>::Item;

pub struct Resize<T: PtyMaster> {
    pty: T,
    rows: c_ushort,
    cols: c_ushort,
}

impl<T: PtyMaster> Resize<T> {
    pub fn new(pty: T, rows: c_ushort, cols: c_ushort) -> Self {
        Self { pty, rows, cols }
    }
}

impl<T: PtyMaster> Future for Resize<T> {
    type Item = ();
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.pty.resize(self.rows, self.cols)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct IO(pub BytesMut);

impl Message for IO {
    type Result = ();
}

impl Into<Binary> for IO {
    fn into(self) -> Binary {
        self.0.into()
    }
}

impl AsRef<[u8]> for IO {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<Binary> for IO {
    fn from(b: Binary) -> Self {
        Self(b.as_ref().into())
    }
}

impl From<String> for IO {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl From<&str> for IO {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

pub struct ChildDied();

impl Message for ChildDied {
    type Result = ();
}
