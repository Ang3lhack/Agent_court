// Untrusted worker: this is the ONLY executable linked with candidate code.
// It executes operations; the separate supervisor owns all test expectations.
use candidate::Counter;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    os::fd::AsFd,
};

fn main() {
    // fd 0 is the supervisor's private socket, not a success-marker stdout.
    let mut channel = File::from(
        io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .expect("worker channel"),
    );
    let input = BufReader::new(channel.try_clone().expect("worker input"));
    let mut counters = [Counter::default(), Counter::default(), Counter::default()];
    for line in input.lines() {
        let line = line.expect("supervisor request");
        let mut fields = line.split_ascii_whitespace();
        let request_id = fields.next().expect("request ID");
        let operation = fields.next().expect("operation");
        let instance: usize = fields
            .next()
            .expect("instance")
            .parse()
            .expect("instance index");
        assert!(fields.next().is_none(), "unexpected request field");
        let counter = &mut counters[instance];
        match operation {
            "value" => {}
            "increment" => {
                counter.increment();
            }
            "reset" => {
                counter.reset();
            }
            _ => panic!("unknown operation"),
        }
        // Enforce the public read-only value(&self) signature too.
        let read: &Counter = counter;
        let value: u64 = read.value();
        writeln!(channel, "{request_id} {value}").expect("worker response");
        channel.flush().expect("worker response flush");
    }
}
