//! shout out noah kennedy:
//! https://gist.github.com/yoshuawuyts/c74b0b344f62133664f36d8192367b97

#![cfg(target_os = "macos")]

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

mod future;
mod runtime;
mod tcp;

use runtime::Poller;
use tcp::AsyncTcpStream;

fn main() -> io::Result<()> {
    // kickoff a simple echo server we can hit for demo purposes
    thread::spawn(run_echo_server);

    // start the poller
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

    let mut buffer = [0; 32];
    let read_future = client.read(&mut buffer);
    let n = poller.block_on(read_future)??;
    assert_eq!(b"hello, world!", &buffer[..n]);
    println!("data validated!");

    // cleanup by removing the event watch
    let close_future = client.disconnect();
    poller.block_on(close_future)?;
    println!("client disconnected!");

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
