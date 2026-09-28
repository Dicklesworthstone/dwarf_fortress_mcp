//! One foreground TCP connection with bounded bytes, calls and cancellation latency.
use super::{BuildCancellation, Result, budget_error, codec, io_error, require};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub(super) const MAX_BYTES: u64 = 512 * 1024;
const MAX_CALLS: u32 = 32;
const SLICE: Duration = Duration::from_millis(100);

/// Limits are selected by source-defined constructors, never by a
/// caller's native method or protocol argument. Furniture defaults stay fixed.
#[derive(Clone, Copy)]
struct Limits {
    bytes: u64,
    calls: u32,
    reply_bytes: usize,
    notification_bytes: u64,
}
const FURNITURE: Limits = Limits {
    bytes: MAX_BYTES,
    calls: MAX_CALLS,
    reply_bytes: codec::MAX_REPLY,
    notification_bytes: MAX_BYTES,
};
const CONSTRUCTION: Limits = Limits {
    bytes: 20 * 1024 * 1024,
    calls: 327,
    reply_bytes: 65536 + 4096,
    notification_bytes: 2 * 1024 * 1024,
};

const ALLOCATION: Limits = Limits {
    bytes: 20 * 1024 * 1024,
    calls: 272,
    reply_bytes: 65536 + 4096,
    notification_bytes: 2 * 1024 * 1024,
};

pub(crate) struct Link {
    socket: TcpStream,
    deadline: Instant,
    remaining_bytes: u64,
    calls: u32,
    cancellation: BuildCancellation,
    limits: Limits,
    notifications_left: u64,
}
impl Link {
    pub(super) fn connect(
        address: SocketAddr,
        timeout: Duration,
        bytes: u64,
        cancellation: BuildCancellation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        Self::connect_profile(address, timeout, bytes, cancellation, check, FURNITURE)
    }
    /// Read-only construction composition uses the same framing, cancellation
    /// slices and shrinking clock with its separately fixed capture allowance.
    pub(crate) fn connect_construction(
        address: SocketAddr,
        timeout: Duration,
        bytes: u64,
        cancellation: BuildCancellation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        Self::connect_profile(address, timeout, bytes, cancellation, check, CONSTRUCTION)
    }
    /// One complete operations/1.4 read for a furniture allocation. No caller
    /// can select a wider profile or renew its connection allowance per page.
    pub(crate) fn connect_furniture_allocation(
        address: SocketAddr,
        timeout: Duration,
        bytes: u64,
        cancellation: BuildCancellation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        Self::connect_profile(address, timeout, bytes, cancellation, check, ALLOCATION)
    }
    fn connect_profile(
        address: SocketAddr,
        timeout: Duration,
        bytes: u64,
        cancellation: BuildCancellation,
        check: &dyn Fn() -> Result<()>,
        limits: Limits,
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
            remaining_bytes: bytes.min(limits.bytes),
            calls: 0,
            cancellation,
            limits,
            notifications_left: limits.notification_bytes,
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
    pub(crate) fn greeting(&mut self, check: &dyn Fn() -> Result<()>) -> Result<()> {
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
        self.frame_bounded(method, request, self.limits.reply_bytes, check)
    }
    pub(crate) fn frame_bounded(
        &mut self,
        method: i16,
        request: &[u8],
        maximum_reply: usize,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<u8>> {
        require(
            request.len() <= codec::MAX_REQUEST && maximum_reply <= self.limits.reply_bytes,
            "native request or reply allowance exceeds its fixed profile",
        )?;
        if self.calls >= self.limits.calls {
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
                n <= if id == -1 { maximum_reply } else { 65536 },
                "native furniture frame exceeds bound",
            )?;
            if id == -3 {
                notifications += 1;
                notification_bytes += n;
                require(
                    notifications <= 8 && notification_bytes <= 262144,
                    "furniture notification allowance exhausted",
                )?;
                self.notifications_left = self
                    .notifications_left
                    .checked_sub(n as u64)
                    .ok_or_else(budget_error)?;
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
