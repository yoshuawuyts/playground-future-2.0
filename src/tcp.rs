use std::io::{self, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, RawFd};

use crate::future::{Future, Interest, Waitable};

pub struct AsyncTcpStream(pub TcpStream);
impl AsRawFd for AsyncTcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsyncTcpStream {
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let client = TcpStream::connect(addr)?;
        client.set_nonblocking(true)?;
        Ok(Self(client))
    }

    pub fn read<'a>(&mut self, data: &'a mut [u8]) -> ReadFuture<'_, 'a> {
        ReadFuture {
            client: self,
            buffer: data,
            output: None,
        }
    }

    pub fn disconnect(self) -> CloseFuture {
        CloseFuture {
            client: self,
            state: CloseFutureState::Pending,
        }
    }
}

pub struct ReadFuture<'a, 'b> {
    client: &'a mut AsyncTcpStream,
    buffer: &'b mut [u8],
    output: Option<io::Result<usize>>,
}

enum Once<T> {
    Empty,
    Once(Option<T>),
}

impl<T> Iterator for Once<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Once::Empty => None,
            Once::Once(opt) => opt.take(),
        }
    }
}

impl<'a, 'b> Future for ReadFuture<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(&mut self, _ready: &[Waitable]) -> impl Iterator<Item = Waitable> {
        // NOTE: this would be significantly nicer to write as `gen fn poll`
        match self.client.0.read(&mut self.buffer) {
            Ok(n) => {
                self.output = Some(Ok(n));
                Once::Empty
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Once::Once(Some(Waitable::Fd(self.client.as_raw_fd(), Interest::Read)))
            }
            Err(e) => {
                self.output = Some(Err(e));
                Once::Empty
            }
        }
    }

    fn take(&mut self) -> Option<Self::Output> {
        self.output.take()
    }
}

enum CloseFutureState {
    Pending,
    Closed,
    Completed,
}

pub struct CloseFuture {
    client: AsyncTcpStream,
    state: CloseFutureState,
}

impl Future for CloseFuture {
    type Output = ();

    fn poll(&mut self, _ready: &[Waitable]) -> impl Iterator<Item = Waitable> {
        match self.state {
            CloseFutureState::Pending => {
                self.state = CloseFutureState::Closed;
                Once::Once(Some(Waitable::Fd(self.client.as_raw_fd(), Interest::Close)))
            }
            CloseFutureState::Closed | CloseFutureState::Completed => Once::Empty,
        }
    }

    fn take(&mut self) -> Option<Self::Output> {
        match self.state {
            CloseFutureState::Closed => {
                self.state = CloseFutureState::Completed;
                Some(())
            }
            CloseFutureState::Pending | CloseFutureState::Completed => None,
        }
    }
}
