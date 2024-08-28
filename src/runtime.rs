use rustix::event::kqueue;
use std::io;
use std::os::fd::{AsFd, OwnedFd, RawFd};
use std::time::Duration;

use crate::future::{Future, Interest, IntoFuture, Waitable};

pub struct Poller {
    queue: OwnedFd,
    events: Vec<kqueue::Event>,
}

impl Poller {
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            queue: kqueue::kqueue()?,
            events: Vec::with_capacity(1),
        })
    }

    // Register the client for interest in read events, and don't wait for events to come in.
    //
    // Safety: we won't polling this after the TcpStream referred to closes, and we delete the
    // event too.
    //
    // Though the rustix docs say that the kqueue must be closed first, this isn't technically true.
    // You could delete the event as well, and failing to do so isn't actually catastrophic - the
    // worst case is more spurious wakes.
    pub fn register_read(&mut self, fd: RawFd) -> io::Result<usize> {
        let flags = kqueue::EventFlags::ADD;
        let udata = 7;
        let event = kqueue::Event::new(kqueue::EventFilter::Read(fd), flags, udata);
        let timeout = None;
        Ok(unsafe { kqueue::kevent(&self.queue, &[event], &mut self.events, timeout)? })
    }

    // Wait for some event to complete
    pub fn wait(&mut self) -> io::Result<usize> {
        // safety: we are not modifying the list, just polling
        Ok(unsafe { kqueue::kevent(self.queue.as_fd(), &[], &mut self.events, None)? })
    }

    // Unregister the client for interest in read events.
    pub fn unregister_read(&mut self, fd: RawFd) -> io::Result<usize> {
        let flags = kqueue::EventFlags::DELETE;
        let udata = 7;
        let event = kqueue::Event::new(kqueue::EventFilter::Read(fd), flags, udata);

        dbg!();

        let mut event_list = vec![];
        let timeout = Some(Duration::ZERO);
        Ok(unsafe { kqueue::kevent(&self.queue, &[event], &mut event_list, timeout)? })
    }

    fn events(&self) -> Vec<Waitable> {
        self.events
            .iter()
            .map(|event| match event.filter() {
                kqueue::EventFilter::Read(fd) => Waitable::Fd(fd, Interest::Read),
                _ => panic!("non-read filter found!"),
            })
            .collect()
    }

    pub fn block_on<Fut: IntoFuture>(&mut self, future: Fut) -> io::Result<Fut::Output> {
        let mut fut = future.into_future();
        loop {
            let mut should_wait = false;
            for waitable in fut.poll(&self.events()) {
                match waitable {
                    Waitable::Fd(fd, Interest::Read) => {
                        should_wait = true;
                        self.register_read(fd)?
                    }
                    Waitable::Fd(fd, Interest::Close) => self.unregister_read(fd)?,
                };
            }
            self.events.clear();

            match should_wait {
                true => self.wait()?,
                false => match fut.take() {
                    Some(output) => return Ok(output),
                    None => panic!("No more events to wait on and no data present"),
                },
            };
        }
    }
}
