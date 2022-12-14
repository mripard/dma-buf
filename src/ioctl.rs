use std::{os::unix::io::RawFd, ptr::addr_of};

use nix::ioctl_write_ptr;

use crate::BufferError;

const DMA_BUF_BASE: u8 = b'b';
const DMA_BUF_IOCTL_SYNC: u8 = 0;

const DMA_BUF_SYNC_READ: u64 = 1 << 0;
const DMA_BUF_SYNC_WRITE: u64 = 1 << 1;
const DMA_BUF_SYNC_START: u64 = 0 << 2;
const DMA_BUF_SYNC_END: u64 = 1 << 2;

#[derive(Default)]
#[repr(C)]
pub struct dma_buf_sync {
    flags: u64,
}

ioctl_write_ptr!(
    dma_buf_ioctl_sync,
    DMA_BUF_BASE,
    DMA_BUF_IOCTL_SYNC,
    dma_buf_sync
);

fn dma_buf_sync(fd: RawFd, flags: u64) -> Result<(), BufferError> {
    let sync = dma_buf_sync { flags };

    unsafe { dma_buf_ioctl_sync(fd, addr_of!(sync)) }
        .map(|_| ())
        .map_err(|e| BufferError::FdAccess {
            reason: e.to_string(),
            source: std::io::Error::from(e),
        })
}

pub fn dma_buf_begin_cpu_read_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ)
}

pub fn dma_buf_begin_cpu_readwrite_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(
        fd,
        DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_READ,
    )
}

pub fn dma_buf_begin_cpu_write_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE)
}

pub fn dma_buf_end_cpu_read_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ)
}

pub fn dma_buf_end_cpu_readwrite_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(
        fd,
        DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_READ,
    )
}

pub fn dma_buf_end_cpu_write_access(fd: RawFd) -> Result<(), BufferError> {
    dma_buf_sync(fd, DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE)
}
