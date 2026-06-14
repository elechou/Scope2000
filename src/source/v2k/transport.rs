use std::io::{Read, Write};
use std::time::Duration;

use thiserror::Error;

use crate::source::TransportEndpoint;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("serial transport: {0}")]
    Serial(#[from] serialport::Error),
    #[error("transport I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("local byte-stream transport is reserved for the external bridge")]
    LocalStreamUnavailable,
}

pub trait ByteTransport: Send {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError>;
    fn read(&mut self, data: &mut [u8]) -> Result<usize, TransportError>;
    fn label(&self) -> &str;
}

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
    label: String,
}

impl SerialTransport {
    pub fn open(port: &str, baud: u32) -> Result<Self, TransportError> {
        let serial = serialport::new(port, baud)
            .timeout(Duration::from_millis(10))
            .open()?;
        Ok(Self {
            port: serial,
            label: format!("{port} @ {baud}"),
        })
    }
}

impl ByteTransport for SerialTransport {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.port.write_all(data)?;
        self.port.flush()?;
        Ok(())
    }

    fn read(&mut self, data: &mut [u8]) -> Result<usize, TransportError> {
        match self.port.read(data) {
            Ok(count) => Ok(count),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(error) => Err(error.into()),
        }
    }

    fn label(&self) -> &str {
        &self.label
    }
}

pub fn open(endpoint: &TransportEndpoint) -> Result<Box<dyn ByteTransport>, TransportError> {
    match endpoint {
        TransportEndpoint::Serial { port, baud } => {
            Ok(Box::new(SerialTransport::open(port, *baud)?))
        }
        TransportEndpoint::LocalByteStream(_) => Err(TransportError::LocalStreamUnavailable),
    }
}

pub fn available_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|port| port.port_name).collect())
        .unwrap_or_default()
}
