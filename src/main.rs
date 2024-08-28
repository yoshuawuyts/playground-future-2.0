use rustix::{
    event::kqueue::{self, Event, EventFilter, EventFlags},
    io::Errno,
};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd, RawFd},
};

use std::net::TcpListener;

#[cfg(target_os = "macos")]
fn main() -> io::Result<()> {
    let mut poller = Poller::new().unwrap();

    // Create a new blocking Tcp listener on a un-allocated port:
    let addr = "127.0.0.1:3456";
    let listener = AsyncTcpListener::bind(addr)?;

    poller.register(Waitable::Fd(listener.as_raw_fd(), Interest::ReadWrite))?;

    // connect to the listener from another thread to trigger an event:
    // std::thread::scope(|s| {
    //     s.spawn(|| {
    //         let _ = TcpStream::connect(addr).unwrap();
    //     });
    // });

    // check for events!
    let events = poller.wait()?;
    assert_eq!(events.len(), 1);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    compile_error!("no impls for non-macos targets quite yet");
}

struct AsyncTcpListener {
    listener: TcpListener,
}

impl AsyncTcpListener {
    pub fn bind(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }
}

impl AsRawFd for AsyncTcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }
}

#[derive(Debug)]
pub enum Waitable {
    /// Registered file descriptor.
    Fd(RawFd, Interest),
    // See: https://github.com/smol-rs/polling/blob/1ee697a750f0b3a3b009294130eed13aa7bc994d/src/kqueue.rs
    // /// Signal.
    // Signal(std::os::raw::c_int, Interest),
    // /// Process ID.
    // Pid(rustix::process::Pid, Interest),
    // /// Timer.
    // Timer(usize, Interest),
}

#[derive(Debug)]
pub enum Interest {
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
    event_list: Vec<kqueue::Event>,
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let queue = kqueue::kqueue()?;
        Ok(Self {
            queue,
            event_list: vec![],
        })
    }
    /// Registers interest in events descripted by `Event` in the given file descriptor referrred to
    /// by `file`.
    pub(crate) fn register(&mut self, event: Waitable) -> io::Result<()> {
        match dbg!(event) {
            Waitable::Fd(fd, interest) => {
                let flags = EventFlags::ADD | EventFlags::RECEIPT | EventFlags::CLEAR;
                match interest {
                    Interest::Read => {
                        dbg!("read");
                        let event = kqueue::Event::new(EventFilter::Read(fd), flags, 1234);
                        unsafe {
                            kqueue::kevent(&self.queue, &[event], &mut self.event_list, None)?
                        };
                    }
                    Interest::Write => {
                        dbg!("write");
                        let event = kqueue::Event::new(EventFilter::Write(fd), flags, 1);
                        unsafe {
                            kqueue::kevent(&self.queue, &[event], &mut self.event_list, None)?
                        };
                    }
                    Interest::ReadWrite => {
                        dbg!("read-write");
                        let events = [
                            kqueue::Event::new(EventFilter::Read(fd), flags, 0),
                            kqueue::Event::new(EventFilter::Write(fd), flags, 0),
                        ];
                        unsafe {
                            kqueue::kevent(&self.queue, &events, &mut self.event_list, None)?
                        };
                    }
                };

                // Check for errors
                let timer = None;
                unsafe { kqueue::kevent(&self.queue, &[], &mut self.event_list, timer)? };
                for event in self.event_list.iter() {
                    check_event_error(event)?;
                }
                self.event_list.clear();

                Ok(())
            }
        }
    }

    pub(crate) fn wait(&mut self) -> io::Result<&[kqueue::Event]> {
        assert_eq!(self.event_list.len(), 0, "doesn't hold any output");

        // wait for all events
        let timer = None;
        unsafe { kqueue::kevent(&self.queue, &[], &mut self.event_list, timer)? };

        // Handle any possible errors
        for event in &self.event_list {
            check_event_error(event)?;
        }

        Ok(&self.event_list)
    }
}

/// Check the returned kevent error.
fn check_event_error(event: &Event) -> io::Result<()> {
    let data = event.data();
    if event.flags().contains(kqueue::EventFlags::ERROR)
            && data != 0
            && data != Errno::NOENT.raw_os_error() as _
            // macOS can sometimes throw EPIPE when registering a file descriptor for a pipe
            // when the other end is already closed, but kevent will still report events on
            // it so there's no point in propagating it...it's valid to register interest in
            // the file descriptor even if the other end of the pipe that it's connected to
            // is closed
            // See: https://github.com/tokio-rs/mio/issues/582
            && data != Errno::PIPE.raw_os_error() as _
    {
        return Err(io::Error::from_raw_os_error(data as _));
    } else {
        Ok(())
    }
}
