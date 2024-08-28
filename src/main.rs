//! shout out noah kennedy:
//! https://gist.github.com/yoshuawuyts/c74b0b344f62133664f36d8192367b97

#![cfg(target_os = "macos")]

use rustix::event::kqueue;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::thread;
use std::time::Duration;

struct Poller {
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
    fn wait(&mut self) -> io::Result<usize> {
        // safety: we are not modifying the list, just polling
        Ok(unsafe { kqueue::kevent(self.queue.as_fd(), &[], &mut self.events, None)? })
    }

    // Unregister the client for interest in read events.
    pub fn unregister_read(&mut self, fd: RawFd) -> io::Result<usize> {
        let flags = kqueue::EventFlags::DELETE;
        let udata = 7;
        let event = kqueue::Event::new(kqueue::EventFilter::Read(fd), flags, udata);

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
}

#[derive(Debug)]
enum Interest {
    Read,
}

#[derive(Debug)]
pub enum Waitable {
    /// Registered file descriptor.
    Fd(RawFd, Interest),
}

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

struct AsyncTcpStream(TcpStream);
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
        ReadFuture { stream: self, data }
    }
}

struct ReadFuture<'a, 'b> {
    stream: &'a mut AsyncTcpStream,
    data: &'b mut [u8],
}

fn block_on<Fut: IntoFuture>(future: Fut) -> io::Result<Fut::Output> {
    let mut fut = future.into_future();
    let mut poller = Poller::open().unwrap();
    loop {
        let mut should_wait = false;
        for waitable in fut.poll(&poller.events()) {
            should_wait = true;
            match waitable {
                Waitable::Fd(fd, Interest::Read) => poller.register_read(fd)?,
            };
        }

        if should_wait {
            poller.wait()?;
        } else {
            match fut.take() {
                Some(output) => return Ok(output),
                None => panic!("Item was already taken somehow?"),
            }
        }
    }
}

fn main() -> io::Result<()> {
    // kickoff a simple echo server we can hit for demo purposes
    thread::spawn(run_echo_server);

    // create the kqueue instance
    let mut poller = Poller::open()?;

    // set up the client
    let mut client = AsyncTcpStream::connect("127.0.0.1:8080")?;

    // we have not written anything yet, this should get EWOULDBLOCK
    assert_eq!(
        std::io::ErrorKind::WouldBlock,
        client.0.read(&mut [0; 32]).unwrap_err().kind()
    );

    // send some data over the wire, and wait 1 second for the server to hopefully echo it back
    client.0.write(b"hello, world!").unwrap();
    thread::sleep(Duration::from_secs(1));

    let mut n = 0;

    // we loop due to spurious events being a possibility, polling may need to be retried
    let mut is_registered = false;
    loop {
        if n == 1 || !is_registered {
            // verify that the event has the user data we specified, this is just to show udata in
            // action
            if is_registered {
                assert_eq!(7, poller.events[0].udata());
            }

            let mut buffer = [0; 32];

            match client.0.read(&mut buffer) {
                Ok(n) => {
                    assert_eq!(b"hello, world!", &buffer[..n]);
                    println!("data validated!");
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // register read interest after the first blocking read call
                    if !is_registered {
                        poller.register_read(client.as_raw_fd())?;
                        is_registered = true;
                    }
                    n = poller.wait()?;
                }
                Err(e) => {
                    panic!("Unexpected error {e:?}");
                }
            }
        }
    }

    // cleanup by removing the event watch
    poller.unregister_read(client.as_raw_fd())?;

    thread::sleep(Duration::from_secs(1));
    println!("done");
    Ok(())
}

fn run_echo_server() {
    let listener = TcpListener::bind("127.0.0.1:8080").unwrap();

    loop {
        let conn = listener.accept().unwrap();
        thread::spawn(move || handle_new_echo_server_connection(conn.0));
    }
}

fn handle_new_echo_server_connection(conn: TcpStream) {
    println!("connection received, waiting 1 sec");
    thread::sleep(Duration::from_secs(1));
    println!("wait over");
    std::io::copy(&mut &conn, &mut &conn).unwrap();
}
