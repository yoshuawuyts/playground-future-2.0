use rustix::event::{
    self,
    kqueue::{self, Event, EventFilter, EventFlags},
};
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd},
    time::Duration,
};

#[cfg(target_os = "macos")]
fn main() {
    println!("hello from macos");
    let poller = Poller::new();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    compile_error!("no impls for non-macos targets quite yet");
}

pub enum Waitable {
    // TODO: rename to "Fd"?
    Network(RawFd, Interest),
}

enum Interest {
    Read,
    Write,
    ReadWrite,
}

/// A representation of an asynchronous computation.
pub trait Future {
    type Output;
    fn poll(&mut self, ready: &[Waitable]) -> impl Iterator<Item = Waitable>;
    fn take(&mut self) -> Option<Self::Output>;
}

/// A conversion into an asynchronous computation.
pub trait IntoFuture {
    type Output;
    type IntoFuture: Future<Output = Self::Output>;
    fn into_future(self) -> Self::IntoFuture;
}

impl<T> IntoFuture for T
where
    T: Future,
{
    type Output = T::Output;
    type IntoFuture = T;
    fn into_future(self) -> Self::IntoFuture {
        self
    }
}

struct Poller {
    queue: OwnedFd,
    events: Vec<kqueue::Event>,
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let queue = kqueue::kqueue()?;
        Ok(Self {
            queue,
            events: vec![],
        })
    }
    /// Registers interest in events descripted by `Event` in the given file descriptor referrred to
    /// by `file`.
    pub(crate) fn register(&mut self, event: Waitable) -> io::Result<()> {
        match event {
            Waitable::Network(fd, interest) => {
                let flags = EventFlags::ADD | EventFlags::RECEIPT | EventFlags::ONESHOT;
                match interest {
                    Interest::Read => {
                        let event = kqueue::Event::new(EventFilter::Read(fd), flags, 0);
                        self.events.push(event);
                    }
                    Interest::Write => {
                        let event = kqueue::Event::new(EventFilter::Write(fd), flags, 0);
                        self.events.push(event);
                    }
                    Interest::ReadWrite => {
                        let event = kqueue::Event::new(EventFilter::Read(fd), flags, 0);
                        self.events.push(event);
                        let event = kqueue::Event::new(EventFilter::Write(fd), flags, 0);
                        self.events.push(event);
                    }
                };

                Ok(())
            }
        }
    }

    pub(crate) fn wait(&self) -> io::Result<Vec<kqueue::Event>> {
        // TODO: we don't need to allocate here actually - see:
        // https://github.com/michaelhelvey/lilfuture/blob/f3bf0c5ff83cc462cb4659471275a95ecd439c39/src/poll.rs#L24
        let mut event_list = vec![];
        let timer = None;
        unsafe { kqueue::kevent(&self.queue, &self.events, &mut event_list, timer)? };

        Ok(event_list)
    }
}
