//! Bounded capture for agent/control processes, never host execution of candidates.
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Completed,
    Timeout,
    OutputLimit,
}

#[derive(Default)]
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exceeded: bool,
    readers_done: u8,
    read_error: bool,
}

pub struct Output {
    pub code: Option<i32>,
    pub stop: Stop,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && self.stop == Stop::Completed
    }
    pub fn diagnostic(&self) -> String {
        format!(
            "exit={:?}, {:?}\nstdout: {}\nstderr: {}",
            self.code,
            self.stop,
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}

fn reader(
    mut pipe: impl Read + Send + 'static,
    capture: Arc<Mutex<Capture>>,
    stdout: bool,
    limit: usize,
) {
    thread::spawn(move || {
        let mut chunk = [0; 4096];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut data = capture.lock().expect("capture lock");
                    let remaining = limit - data.stdout.len() - data.stderr.len();
                    data.exceeded |= n > remaining;
                    let target = if stdout {
                        &mut data.stdout
                    } else {
                        &mut data.stderr
                    };
                    target.extend_from_slice(&chunk[..n.min(remaining)]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    capture.lock().expect("capture lock").read_error = true;
                    break;
                }
            }
        }
        capture.lock().expect("capture lock").readers_done += 1;
    });
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    command.stdin(Stdio::null());
    run_bounded(command, timeout, OUTPUT_LIMIT)
}

pub fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<Output, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("no se pudo iniciar el proceso: {error}"))?;
    let capture = Arc::new(Mutex::new(Capture::default()));
    reader(
        child.stdout.take().expect("stdout pipe"),
        capture.clone(),
        true,
        limit,
    );
    reader(
        child.stderr.take().expect("stderr pipe"),
        capture.clone(),
        false,
        limit,
    );
    let start = Instant::now();
    let (status, mut stop) = loop {
        let exceeded = capture.lock().expect("capture lock").exceeded;
        let stop = if exceeded {
            Stop::OutputLimit
        } else if start.elapsed() >= timeout {
            Stop::Timeout
        } else {
            Stop::Completed
        };
        if stop != Stop::Completed {
            stop_tree(&mut child);
            let status = child
                .wait()
                .map_err(|error| format!("no se pudo recoger el proceso: {error}"))?;
            break (status, stop);
        }
        match child.try_wait() {
            Ok(Some(status)) => break (status, Stop::Completed),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                stop_tree(&mut child);
                let _ = child.wait();
                return Err(format!("no se pudo consultar el proceso: {error}"));
            }
        }
    };
    // A completed parent must not leave command/tool descendants running.
    stop_tree(&mut child);
    // A descendant holding a pipe must not cause an unbounded reader join.
    let drain = Instant::now();
    while capture.lock().expect("capture lock").readers_done < 2 {
        if drain.elapsed() >= Duration::from_millis(250) {
            return Err("el proceso terminó pero no cerró sus tuberías de salida".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    let mut data = capture.lock().expect("capture lock");
    if data.read_error {
        return Err("falló la lectura de salida del proceso".into());
    }
    if data.exceeded {
        stop = Stop::OutputLimit;
    }
    Ok(Output {
        code: status.code(),
        stop,
        stdout: std::mem::take(&mut data.stdout),
        stderr: std::mem::take(&mut data.stderr),
    })
}

fn stop_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The child owns a fresh process group. Use an absolute control utility,
        // not an agent-controlled PATH or a shell, to stop its descendants too.
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}
