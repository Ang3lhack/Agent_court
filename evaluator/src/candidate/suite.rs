// Independent supervisor: NEVER link this process with the candidate library.
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, Stdio},
};

struct Worker {
    child: Child,
    input: UnixStream,
    output: BufReader<UnixStream>,
    entropy: File,
}

fn infrastructure(error: std::io::Error) -> ! {
    eprintln!("supervisor infrastructure: {error}");
    std::process::exit(20);
}

impl Worker {
    fn start() -> Self {
        let entropy = File::open("/dev/urandom").unwrap_or_else(|e| infrastructure(e));
        // A private bidirectional socket on fd 0 keeps ordinary stdout/stderr
        // separate from replies, under the existing Docker output limit.
        let (input, peer) = UnixStream::pair().unwrap_or_else(|e| infrastructure(e));
        let output = BufReader::new(input.try_clone().unwrap_or_else(|e| infrastructure(e)));
        let peer: OwnedFd = peer.into();
        let child = Command::new("/work/worker")
            .stdin(Stdio::from(peer))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| infrastructure(e));
        Self {
            input,
            output,
            child,
            entropy,
        }
    }

    fn call(&mut self, operation: &str, instance: usize) -> u64 {
        // Fresh request IDs reject preprinted/replayed responses. They are not
        // authentication against arbitrary code inside the worker (see README).
        let mut bytes = [0; 16];
        self.entropy
            .read_exact(&mut bytes)
            .unwrap_or_else(|e| infrastructure(e));
        let request_id = format!("{:032x}", u128::from_le_bytes(bytes));
        writeln!(self.input, "{request_id} {operation} {instance}")
            .expect("worker terminated before receiving the next operation");
        self.input.flush().expect("worker input closed prematurely");

        // A worker without a newline cannot grow the supervisor's memory.
        let mut response = String::new();
        self.output
            .by_ref()
            .take(64)
            .read_line(&mut response)
            .expect("invalid worker response");
        assert!(
            response.ends_with('\n'),
            "missing, premature or oversized worker response for {operation}"
        );
        let (actual_id, value) = response
            .trim_end_matches('\n')
            .split_once(' ')
            .expect("malformed worker response");
        assert_eq!(
            actual_id, request_id,
            "unsolicited or replayed worker response"
        );
        value.parse().expect("worker value must be a u64")
    }

    fn finish(mut self) {
        // EOF asks the worker to shut down normally, including with cloned FDs.
        self.input
            .shutdown(Shutdown::Write)
            .expect("worker shutdown");
        assert_eq!(
            self.output.read(&mut [0]).expect("worker final output"),
            0,
            "unsolicited trailing worker output"
        );
        assert!(
            self.child.wait().expect("worker wait").success(),
            "worker did not shut down successfully after all checks"
        );
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Also runs on a failed assertion; the outer Docker deadline remains.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn main() {
    let mut worker = Worker::start();
    assert_eq!(worker.call("value", 0), 0_u64, "initial value");

    for expected in 1_u64..=100 {
        assert_eq!(
            worker.call("increment", 1),
            expected,
            "successive increments"
        );
    }

    for _ in 0..10 {
        assert_eq!(
            worker.call("value", 2),
            0_u64,
            "value must not mutate the counter"
        );
    }
    for _ in 0..10 {
        assert_eq!(
            worker.call("value", 1),
            100_u64,
            "repeated value after increments"
        );
    }

    assert_eq!(worker.call("reset", 1), 0_u64, "reset after increments");
    assert_eq!(worker.call("reset", 1), 0_u64, "reset is idempotent");
    assert_eq!(worker.call("increment", 1), 1_u64, "increment after reset");
    assert_eq!(worker.call("value", 0), 0_u64, "independent instances");
    worker.finish();
}
