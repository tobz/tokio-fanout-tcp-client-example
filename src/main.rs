extern crate futures;
extern crate tokio;
extern crate tokio_io;

use futures::future::{lazy, ok};
use futures::sync::mpsc::unbounded;
use tokio::prelude::*;
use tokio::io;
use tokio::net::TcpListener;
use tokio_io::codec::{FramedRead, LinesCodec};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

fn main() {
    tokio::run(lazy(|| {
        // Create our listener.
        let addr = "127.0.0.1:6142".parse::<SocketAddr>().unwrap();
        let listener = TcpListener::bind(&addr).unwrap();

        // Keep a map of connection ID to connection TX channel.
        let state = Arc::new(Mutex::new(HashMap::new()));
        let connection_id = Arc::new(AtomicUsize::new(0));

        // Now for each connection, run a writer/reader that pulls messages from the client and
        // sends them to the given client.  Messages are broadcasted to all other connections.  If
        // the message is "quit", then we close that specific connection.
        listener.incoming()
            .map_err(|err| println!("caught error while accepting connection: {:?}", err))
            .for_each(move |conn| {
                // Grab a connection ID for this new connection.
                let connection_id = connection_id.fetch_add(1, Ordering::SeqCst);
                println!("new connection, id is {}", connection_id);

                // Create a channel that lets us send to this connection.
                let (tx, rx) = unbounded();

                // Now that we have our pathway to send/signal our client, store that.
                {
                    let state_guard = state.lock();
                    state_guard.unwrap().insert(connection_id, tx);
                }

                // Create a line-based codec over our connection so we can read simple messages.
                let (conn_rx, conn_tx) = conn.split();
                let lines_rx = FramedRead::new(conn_rx, LinesCodec::new());

                // Now, read all messages that we get from this connection and fan them out to
                // other connections, but not ourselves.
                let reader_conn_state = state.clone();
                let reader = lines_rx.for_each(move |msg| {
                    println!("received new message!");
                    let self_conn_id = connection_id;

                    // Loop through all connections in the state, and send them this message, so
                    // long as the connection isn't ours.
                    let conn_state = reader_conn_state.lock().unwrap();
                    for (peer_id, peer_tx) in conn_state.iter() {
                        if peer_id != &self_conn_id {
                            println!("pushing msg to peer...");
                            let msg_copy = msg.clone();
                            peer_tx.unbounded_send(msg_copy).unwrap();
                        }
                    }

                    ok(())
                })
                .map_err(|_| ());

                // Now, write all messages that come into our write queue.
                let writer = rx.fold(conn_tx, |w, msg| {
                    println!("writing msg!");
                    io::write_all(w, msg)
                        .map(|(w, _)| w)
                        .map_err(|_| ())
                });

                // Now, drive both our reader and our writer, making sure that we end execution if
                // our close signal fires.
                let wrapped = reader.join(writer)
                    .map(|(_, _)| ())
                    .map_err(|_| ());
                tokio::spawn(wrapped)
            })
    }))
}
