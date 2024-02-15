use std::os::fd::BorrowedFd;

use rustix::{
    io::Errno,
    ioctl::{ioctl, Setter, WriteOpcode},
};

use crate::BufferError;

const DMA_BUF_BASE: u8 = b'b';
const DMA_BUF_IOCTL_SYNC: u8 = 0;

const DMA_BUF_SYNC_READ: u64 = 1 << 0;
const DMA_BUF_SYNC_WRITE: u64 = 1 << 1;
const DMA_BUF_SYNC_START: u64 = 0 << 2;
const DMA_BUF_SYNC_END: u64 = 1 << 2;

#[derive(Default)]
#[repr(C)]
struct dma_buf_sync {
    flags: u64,
}

fn dma_buf_sync_ioctl(fd: BorrowedFd<'_>, flags: u64) -> Result<(), Errno> {
    type Opcode = WriteOpcode<DMA_BUF_BASE, DMA_BUF_IOCTL_SYNC, dma_buf_sync>;

    let sync = dma_buf_sync { flags };

    // SAFETY: This function is unsafe because the opcode has to be valid, and the value type must
    // match. We have checked those, so we're good.
    let ioctl_type = unsafe { Setter::<Opcode, dma_buf_sync>::new(sync) };

    // SAFETY: This function is unsafe because the driver isn't guaranteed to implement the ioctl,
    // and to implement it properly. We don't have much of a choice and still have to trust the
    // kernel there.
    unsafe { ioctl(fd, ioctl_type) }
}

fn dma_buf_sync(fd: BorrowedFd<'_>, flags: u64) -> Result<(), BufferError> {
    dma_buf_sync_ioctl(fd, flags).map_err(|e| BufferError::FdAccess {
        reason: e.to_string(),
        source: std::io::Error::from(e),
    })
}

pub(crate) fn dma_buf_begin_cpu_read_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ)
}

pub(crate) fn dma_buf_begin_cpu_readwrite_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(
        fd,
        DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_READ,
    )
}

pub(crate) fn dma_buf_begin_cpu_write_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE)
}

pub(crate) fn dma_buf_end_cpu_read_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ)
}

pub(crate) fn dma_buf_end_cpu_readwrite_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(
        fd,
        DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_READ,
    )
}

pub(crate) fn dma_buf_end_cpu_write_access(fd: BorrowedFd<'_>) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE)
}
