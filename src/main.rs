use rustix::event::kqueue::{self, Event, EventFlags};
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd},
    time::Duration,
};

#[cfg(target_os = "macos")]
fn main() {
    println!("hello from macos");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    compile_error!("no impls for non-macos targets quite yet");
}

pub enum Waitable {
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
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let queue = kqueue::kqueue()?;
        Ok(Self { queue })
    }
    /// Registers interest in events descripted by `Event` in the given file descriptor referrred to
    /// by `file`.
    pub(crate) fn register(&self, event: Waitable) -> io::Result<()> {
        match event {
            Waitable::Network(fd, interest) => {
                let (read_flags, write_flags) = match interest {
                    Interest::Read => (EventFlags::ADD, EventFlags::DELETE),
                    Interest::Write => (EventFlags::DELETE, EventFlags::ADD),
                    Interest::ReadWrite => (EventFlags::ADD, EventFlags::ADD),
                };
                // Because all of our events are ONESHOT we don't need to provide a de-registration API.
                let common_file_flags = EventFlags::RECEIPT | EventFlags::ONESHOT;

                // TODO: this is pretty goofy: if we don't have a specific kind
                // of interest, we shouldn't create an event.
                let changelist = [
                    kqueue::Event::new(
                        kqueue::EventFilter::Read(fd),
                        common_file_flags | read_flags,
                        0,
                    ),
                    kqueue::Event::new(
                        kqueue::EventFilter::Write(fd),
                        common_file_flags | write_flags,
                        0,
                    ),
                ];

                // Create our buffer on the stack that kqueue will use to write responses into for each
                // event that we pass in our changelist
                unsafe {
                    register_events(&self.queue, &changelist, None)?;
                };

                Ok(())
            }
            _ => panic!("unknown even type"),
        }
    }

    pub(crate) fn wait(&self, events: &mut Events) -> io::Result<()> {
        unsafe { kqueue::kevent(&self.queue, &[], &mut events.eventlist, None)? };

        Ok(())
    }
}
