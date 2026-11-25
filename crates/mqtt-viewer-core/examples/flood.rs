//! Publishes synthetic sensor data, for load-testing the app.
//!
//! ```text
//! cargo run --release -p mqtt-viewer-core --example flood -- [host] [port] [topics] [rate] [seconds]
//! ```
//!
//! Defaults: `127.0.0.1 1883 50000 5000 30`, i.e. 50 000 topics at 5 000 messages per second for 30 seconds.

use std::error::Error;
use std::time::{Duration, Instant};

use bytes::Bytes;
use mqtt_viewer_core::config::ConnectionProfile;
use mqtt_viewer_core::connection::{Command, Connection, ConnectionState, Event};
use mqtt_viewer_core::message::PublishRequest;

/// Messages are sent in slices this long.
const TICK: Duration = Duration::from_millis(10);

fn argument<T: std::str::FromStr>(index: usize, default: T) -> Result<T, Box<dyn Error>>
where
    T::Err: Error + 'static,
{
    std::env::args().nth(index).map_or(Ok(default), |text| text.parse().map_err(Into::into))
}

fn main() -> Result<(), Box<dyn Error>> {
    let host = argument(1, "127.0.0.1".to_owned())?;
    let port: u16 = argument(2, 1883)?;
    let topics: u64 = argument(3, 50_000)?;
    let rate: u64 = argument(4, 5_000)?;
    let seconds: u64 = argument(5, 30)?;

    let connection = Connection::spawn(|| {})?;
    let profile = ConnectionProfile { host, port, subscriptions: Vec::new(), ..ConnectionProfile::default() };
    println!("connecting to {}", profile.url());
    connection.send(Command::Connect(Box::new(profile)));
    loop {
        match connection.wait_event(Duration::from_secs(10)) {
            Some(Event::State(ConnectionState::Connected)) => break,
            Some(Event::Error(error) | Event::State(ConnectionState::Reconnecting { error, .. })) => {
                return Err(error.into());
            }
            Some(_) => {}
            None => return Err("timed out waiting for the broker".into()),
        }
    }

    println!("publishing {rate} msg/s over {topics} topics for {seconds} s");
    let started = Instant::now();
    let mut sent = 0_u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        // Send whatever the elapsed time calls for, so a slow tick does not lower the average rate.
        let due = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX).saturating_mul(rate) / 1000;
        while sent < due {
            let index = sent % topics.max(1);
            let request = PublishRequest {
                topic: format!("flood/site{}/device{}/sensor{index}", index % 10, index % 500),
                payload: Bytes::from(format!(r#"{{"value":{},"seq":{sent},"unit":"C"}}"#, 20 + sent % 10)),
                ..PublishRequest::default()
            };
            connection.send(Command::Publish(request));
            sent += 1;
        }
        std::thread::sleep(TICK);
    }
    // Give the worker time to drain its queue before the connection is dropped.
    std::thread::sleep(Duration::from_secs(1));
    println!("sent {sent} messages in {:.1} s", started.elapsed().as_secs_f64());
    Ok(())
}
