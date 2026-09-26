//! One foreground TCP connection with bounded bytes, calls and cancellation latency.
use super::{BuildCancellation, Result, budget_error, codec, io_error, require};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub(super) const MAX_BYTES: u64 = 512 * 1024;
const MAX_CALLS: u32 = 32;
const SLICE: Duration = Duration::from_millis(100);

pub(super) struct Link {
    socket: TcpStream,
    deadline: Instant,
    remaining_bytes: u64,
    calls: u32,
    cancellation: BuildCancellation,
}
impl Link {
    pub(super) fn connect(
        address: SocketAddr,
        timeout: Duration,
        bytes: u64,
        cancellation: BuildCancellation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        cancellation.check()?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(budget_error)?;
        let socket = TcpStream::connect_timeout(&address, timeout.min(SLICE)).map_err(io_error)?;
        socket.set_nodelay(true).map_err(io_error)?;
        let out = Self {
            socket,
            deadline,
            remaining_bytes: bytes.min(MAX_BYTES),
            calls: 0,
            cancellation,
        };
        out.check(check)?;
        Ok(out)
    }
    pub(super) fn narrow(&mut self, timeout: Duration, bytes: u64) -> Result<()> {
        let proposed = Instant::now()
            .checked_add(timeout)
            .ok_or_else(budget_error)?;
        self.deadline = self.deadline.min(proposed);
        self.remaining_bytes = self.remaining_bytes.min(bytes);
        self.remaining()?;
        Ok(())
    }
    fn remaining(&self) -> Result<Duration> {
        self.cancellation.check()?;
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(budget_error)?;
        if left < Duration::from_millis(1) {
            return Err(budget_error());
        }
        Ok(left)
    }
    fn check(&self, permission: &dyn Fn() -> Result<()>) -> Result<Duration> {
        permission()?;
        self.remaining()
    }
    fn reserve(&mut self, count: usize) -> Result<()> {
        self.remaining()?;
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(count as u64)
            .ok_or_else(budget_error)?;
        Ok(())
    }
    fn send(&mut self, mut data: &[u8], check: &dyn Fn() -> Result<()>) -> Result<()> {
        self.reserve(data.len())?;
        while !data.is_empty() {
            self.socket
                .set_write_timeout(Some(self.check(check)?.min(SLICE)))
                .map_err(io_error)?;
            match self.socket.write(data) {
                Ok(0) => return Err(io_error(io::Error::from(io::ErrorKind::WriteZero))),
                Ok(n) => data = &data[n..],
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(io_error(e)),
            }
        }
        self.check(check)?;
        Ok(())
    }
    fn receive(&mut self, mut data: &mut [u8], check: &dyn Fn() -> Result<()>) -> Result<()> {
        self.reserve(data.len())?;
        while !data.is_empty() {
            self.socket
                .set_read_timeout(Some(self.check(check)?.min(SLICE)))
                .map_err(io_error)?;
            match self.socket.read(data) {
                Ok(0) => return Err(io_error(io::Error::from(io::ErrorKind::UnexpectedEof))),
                Ok(n) => data = &mut data[n..],
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(io_error(e)),
            }
        }
        self.check(check)?;
        Ok(())
    }
    pub(super) fn greeting(&mut self, check: &dyn Fn() -> Result<()>) -> Result<()> {
        self.send(b"DFHack?\n\x01\0\0\0", check)?;
        let mut reply = [0; 12];
        self.receive(&mut reply, check)?;
        require(&reply == b"DFHack!\n\x01\0\0\0", "invalid DFHack greeting")
    }
    pub(super) fn frame(
        &mut self,
        method: i16,
        request: &[u8],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<u8>> {
        require(
            request.len() <= codec::MAX_REQUEST,
            "furniture request exceeds 2 KiB",
        )?;
        if self.calls >= MAX_CALLS {
            return Err(budget_error());
        }
        self.calls += 1;
        let mut wire = codec::header(method, request.len() as i32).to_vec();
        wire.extend_from_slice(request);
        self.send(&wire, check)?;
        let mut notifications = 0;
        let mut notification_bytes = 0;
        loop {
            let mut h = [0; 8];
            self.receive(&mut h, check)?;
            let id = i16::from_le_bytes([h[0], h[1]]);
            let n = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
            require(
                matches!(id, -1 | -3) && n >= 0,
                "invalid native furniture frame",
            )?;
            let n = n as usize;
            require(
                n <= if id == -1 { codec::MAX_REPLY } else { 65536 },
                "native furniture frame exceeds bound",
            )?;
            if id == -3 {
                notifications += 1;
                notification_bytes += n;
                require(
                    notifications <= 8 && notification_bytes <= 262144,
                    "furniture notification allowance exhausted",
                )?;
            }
            let mut data = vec![0; n];
            self.receive(&mut data, check)?;
            if id == -1 {
                return Ok(data);
            }
        }
    }
    pub(super) fn fence(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
