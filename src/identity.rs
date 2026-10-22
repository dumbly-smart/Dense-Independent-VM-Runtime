use serde::Serialize;
use std::fs::File;
use std::io::Read;

/// Random identifiers come from the host kernel; entropy payloads are never
/// serialized into result metadata. Reusing clone indices across runs is safe.
#[derive(Serialize)]
pub struct CloneIdentity {
    pub machine_id: String,
    pub mac: String,
    #[serde(skip)]
    entropy: [u8; 32],
}

impl CloneIdentity {
    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0u8; 54];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| format!("host entropy unavailable: {e}"))?;
        let machine_id = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
        bytes[16] = (bytes[16] & 0xfc) | 2;
        let mac = bytes[16..22]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        let mut entropy = [0; 32];
        entropy.copy_from_slice(&bytes[22..]);
        Ok(Self {
            machine_id,
            mac,
            entropy,
        })
    }

    pub fn entropy_hex(&self) -> String {
        self.entropy.iter().map(|b| format!("{b:02x}")).collect()
    }
}
