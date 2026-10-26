use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub struct GuestSerial {
    stream: UnixStream,
    buffered: Vec<u8>,
}

impl GuestSerial {
    pub fn connect(path: &Path, timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() {
            return Err("serial connect timeout must be positive".into());
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or("serial connect timeout is too large")?;
        loop {
            match UnixStream::connect(path) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .map_err(|error| format!("set guest serial read timeout: {error}"))?;
                    stream
                        .set_write_timeout(Some(timeout))
                        .map_err(|error| format!("set guest serial write timeout: {error}"))?;
                    return Ok(Self::from_stream(stream));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    if Instant::now() >= deadline {
                        return Err(format!("connect guest serial {}: {error}", path.display()));
                    }
                    thread::sleep(
                        Duration::from_millis(20)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(error) => {
                    return Err(format!("connect guest serial {}: {error}", path.display()));
                }
            }
        }
    }

    fn from_stream(stream: UnixStream) -> Self {
        Self {
            stream,
            buffered: Vec::new(),
        }
    }

    pub fn command(
        &mut self,
        command: &str,
        expected: &str,
        timeout: Duration,
    ) -> Result<String, String> {
        if command.contains(['\n', '\r']) || expected.is_empty() || timeout.is_zero() {
            return Err("invalid guest serial command, expected prefix, or timeout".into());
        }
        self.stream
            .write_all(format!("{command}\n").as_bytes())
            .map_err(|error| format!("write guest serial command: {error}"))?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or("guest serial command timeout is too large")?;
        loop {
            let line = self.read_line(deadline)?;
            let Some(reply) = line.strip_prefix("DENSEVM ") else {
                continue;
            };
            if reply.starts_with("ERROR") {
                return Err(format!("guest command {command:?} returned {reply:?}"));
            }
            if reply.starts_with(expected) {
                return Ok(reply.to_string());
            }
        }
    }

    fn read_line(&mut self, deadline: Instant) -> Result<String, String> {
        loop {
            if let Some(end) = self.buffered.iter().position(|byte| *byte == b'\n') {
                let line = self.buffered.drain(..=end).collect::<Vec<_>>();
                return Ok(String::from_utf8_lossy(&line[..line.len() - 1])
                    .trim_end_matches('\r')
                    .to_string());
            }
            if self.buffered.len() > 64 * 1024 {
                return Err("guest serial line exceeds 64 KiB".into());
            }
            if Instant::now() >= deadline {
                return Err("guest serial response timed out".into());
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("guest serial disconnected".into()),
                Ok(length) => self.buffered.extend_from_slice(&chunk[..length]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("read guest serial: {error}")),
            }
        }
    }
}

pub fn prepare_clone_identity(
    serial: &mut GuestSerial,
    identity: &crate::identity::CloneIdentity,
) -> Result<String, String> {
    let timeout = Duration::from_secs(20);
    let machine_id = &identity.machine_id;
    let vmgenid = serial.command("vmgenid-reaction-read", "VMGENID-REACTION", timeout)?;
    serial.command(
        &format!("entropy-add {}", identity.entropy_hex()),
        "RNG-RESEEDED",
        timeout,
    )?;
    serial.command(
        &format!("identity-set {machine_id}"),
        "IDENTITY-SET ",
        timeout,
    )?;
    let guest_identity = serial.command("identity-read", "IDENTITY ", timeout)?;
    let expected_identity = format!(
        "IDENTITY {machine_id} densevm-{} RNG-RESEEDED",
        &machine_id[..12]
    );
    if guest_identity != expected_identity {
        return Err(format!(
            "guest identity verification failed: expected {expected_identity:?}, found {guest_identity:?}"
        ));
    }
    let ssh_identity = serial.command("ssh-identity-read", "SSH-IDENTITY ", timeout)?;
    let dhcp_identity = serial.command("dhcp-identity-read", "DHCP-IDENTITY ", timeout)?;
    Ok(format!(
        "{guest_identity} | {ssh_identity} | {dhcp_identity} | {vmgenid}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::thread;

    #[test]
    fn prepares_entropy_and_unique_machine_identity_before_networking() {
        let identity = crate::identity::CloneIdentity::generate().unwrap();
        let expected_machine_id = identity.machine_id.clone();
        let (client, server) = UnixStream::pair().unwrap();
        let responder = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut writer = server;
            for (expected, reply) in [
                (
                    "vmgenid-reaction-read",
                    "DENSEVM VMGENID-REACTION events=1\n".to_string(),
                ),
                ("entropy-add ", "DENSEVM RNG-RESEEDED\n".to_string()),
                (
                    "identity-set ",
                    format!("DENSEVM IDENTITY-SET {expected_machine_id}\n"),
                ),
                (
                    "identity-read",
                    format!(
                        "DENSEVM IDENTITY {expected_machine_id} densevm-{} RNG-RESEEDED\n",
                        &expected_machine_id[..12]
                    ),
                ),
                (
                    "ssh-identity-read",
                    "DENSEVM SSH-IDENTITY unique-host-key\n".to_string(),
                ),
                (
                    "dhcp-identity-read",
                    "DENSEVM DHCP-IDENTITY unique-client\n".to_string(),
                ),
            ] {
                let mut command = String::new();
                reader.read_line(&mut command).unwrap();
                assert!(command.trim_end().starts_with(expected));
                writer.write_all(reply.as_bytes()).unwrap();
            }
        });
        let mut serial = GuestSerial::from_stream(client);
        let report = prepare_clone_identity(&mut serial, &identity).unwrap();
        assert!(report.contains(&identity.machine_id));
        assert!(report.contains("unique-host-key"));
        assert!(report.contains("unique-client"));
        responder.join().unwrap();
    }

    #[test]
    fn rejects_guest_identity_errors_instead_of_reporting_success() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let responder = thread::spawn(move || {
            let mut command = String::new();
            BufReader::new(server.try_clone().unwrap())
                .read_line(&mut command)
                .unwrap();
            server
                .write_all(b"DENSEVM ERROR identity denied\n")
                .unwrap();
        });
        let mut serial = GuestSerial::from_stream(client);
        assert!(
            serial
                .command(
                    "identity-set deadbeef",
                    "IDENTITY-SET ",
                    Duration::from_secs(1)
                )
                .is_err()
        );
        responder.join().unwrap();
    }
}
