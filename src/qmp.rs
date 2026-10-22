use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub struct Qmp {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Qmp {
    pub fn connect(path: &Path, timeout: Duration) -> Result<Self, String> {
        let writer = UnixStream::connect(path).map_err(|e| format!("QMP connect: {e}"))?;
        writer
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        writer
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        let reader = BufReader::new(writer.try_clone().map_err(|e| e.to_string())?);
        let mut qmp = Self {
            reader,
            writer,
            next_id: 1,
        };
        let greeting = qmp.read_json()?;
        if greeting.get("QMP").is_none() {
            return Err(format!("invalid QMP greeting: {greeting}"));
        }
        qmp.execute("qmp_capabilities", None)?;
        Ok(qmp)
    }

    fn read_json(&mut self) -> Result<Value, String> {
        crate::interrupt::check()?;
        let mut line = String::new();
        let length = self
            .reader
            .read_line(&mut line)
            .map_err(|e| format!("QMP read: {e}"))?;
        if length == 0 {
            return Err("QMP closed connection".to_string());
        }
        serde_json::from_str(&line).map_err(|e| format!("invalid QMP JSON: {e}"))
    }

    pub fn execute(&mut self, command: &str, arguments: Option<Value>) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut message = json!({"execute": command, "id": id});
        if let Some(arguments) = arguments {
            message["arguments"] = arguments;
        }
        let mut bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .map_err(|e| format!("QMP write: {e}"))?;
        loop {
            let response = self.read_json()?;
            if response.get("event").is_some() {
                continue;
            }
            if let Some(response_id) = response.get("id")
                && response_id != id
            {
                return Err(format!("QMP reply id mismatch: {response}"));
            }
            if let Some(error) = response.get("error") {
                return Err(format!("QMP {command}: {error}"));
            }
            return response
                .get("return")
                .cloned()
                .ok_or_else(|| format!("invalid QMP reply: {response}"));
        }
    }
}
