//! shout out noah kennedy:
//! https://gist.github.com/yoshuawuyts/c74b0b344f62133664f36d8192367b97

#![cfg(target_os = "macos")]

use rustix::event::kqueue;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
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
        // add a notification for read-readiness
        let notif = kqueue::EventFilter::Read(fd);
        // add a level-triggered event
        //
        // add EV_CLEAR too to make it edge-triggered, which is generally what you want in
        // practice, but that is a discussion for another time
        let flags = kqueue::EventFlags::ADD;
        // 7 seems like a nice number to assert we get back!
        let udata = 7;
        let event = kqueue::Event::new(notif, flags, udata);

        // pass in no timeout, and wait indefinitely for events
        let timeout = None;
        let event_count =
            unsafe { kqueue::kevent(&self.queue, &[event], &mut self.events, timeout)? };
        Ok(event_count)
    }

    fn wait(&mut self) -> io::Result<usize> {
        // safety: we are not modifying the list, just polling
        let event_count =
            unsafe { kqueue::kevent(self.queue.as_fd(), &[], &mut self.events, None)? };
        Ok(event_count)
    }

    pub fn unregister_read(&mut self, fd: RawFd) -> io::Result<usize> {
        let change_list = &[kqueue::Event::new(
            kqueue::EventFilter::Read(fd),
            // remove the event
            kqueue::EventFlags::DELETE,
            7,
        )];

        // we are not waiting on events this time, no need to pass in a real buffer
        let mut event_list = Vec::new();

        // dont block
        let timeout = Some(Duration::ZERO);
        let event_count =
            unsafe { kqueue::kevent(&self.queue, change_list, &mut event_list, timeout)? };
        Ok(event_count)
    }
}

impl AsFd for Poller {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.queue.as_fd()
    }
}

fn main() -> io::Result<()> {
    // kickoff a simple echo server we can hit for demo purposes
    thread::spawn(run_echo_server);

    // create the kqueue instance
    let mut poller = Poller::open()?;

    // set up the client
    let mut client = TcpStream::connect("127.0.0.1:8080")?;
    client.set_nonblocking(true)?;

    // we have not written anything yet, this should get EWOULDBLOCK
    assert_eq!(
        std::io::ErrorKind::WouldBlock,
        client.read(&mut [0; 32]).unwrap_err().kind()
    );

    // send some data over the wire, and wait 1 second for the server to hopefully echo it back
    client.write(b"hello, world!").unwrap();
    thread::sleep(Duration::from_secs(1));

    let mut event_count = poller.register_read(client.as_raw_fd())?;

    // we loop due to spurious events being a possibility, polling may need to be retried
    loop {
        if event_count == 1 {
            // verify that the event has the user data we specified, this is just to show udata in
            // action
            assert_eq!(7, poller.events[0].udata());

            let mut buffer = [0; 32];

            match client.read(&mut buffer) {
                Ok(n) => {
                    assert_eq!(b"hello, world!", &buffer[..n]);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    event_count = poller.wait()?
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
