// Copyright 2020-2021, Cerno
// Licensed under the MIT License
// See the LICENSE file or <http://opensource.org/licenses/MIT>

#![doc = include_str!("../README.md")]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::cargo)]

use core::fmt;
use std::{
    convert::TryInto,
    os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

use ioctl::{
    dma_buf_begin_cpu_read_access, dma_buf_begin_cpu_readwrite_access,
    dma_buf_begin_cpu_write_access, dma_buf_end_cpu_read_access, dma_buf_end_cpu_readwrite_access,
    dma_buf_end_cpu_write_access,
};
use log::debug;
use memmap::MmapMut;
use nix::sys::stat::fstat;

mod ioctl;

/// Error type to map a [`DmaBuf`]
#[derive(thiserror::Error, Debug)]
pub enum MapError {
    /// An Error occurred while accessing the buffer file descriptor
    #[error("Could not access the buffer file descriptor: {reason}")]
    FdAccess {
        /// Description of the Error
        reason: String,

        /// Source of the Error
        source: std::io::Error,
    },

    /// An Error occurred while mapping the buffer file descriptor
    #[error("Could not map the buffer file descriptor: {reason}")]
    MappingFailed {
        /// Description of the Error
        reason: String,

        /// Source of the Error
        source: std::io::Error,
    },
}

/// A DMA-Buf buffer
#[derive(Debug)]
pub struct DmaBuf(OwnedFd);

impl DmaBuf {
    /// Maps a `DmaBuf` for the CPU to access it
    ///
    /// # Panics
    ///
    /// If the buffer size reported by the kernel (`i64`) cannot fit into an `usize`.
    ///
    /// # Errors
    ///
    /// Will return an error if either the Buffer's length can't be retrieved, or if the mmap call
    /// fails.
    pub fn memory_map(self) -> Result<MappedDmaBuf, MapError> {
        let raw_fd = self.as_raw_fd();

        debug!("Mapping DMA-Buf buffer with File Descriptor {:#?}", self.0);

        let stat = fstat(raw_fd).map_err(|e| MapError::FdAccess {
            reason: e.to_string(),
            source: std::io::Error::from(e),
        })?;

        let len = stat.st_size.try_into().unwrap();
        debug!("Valid buffer, size {}", len);

        let mmap = unsafe { MmapMut::map_mut(raw_fd) }.map_err(|e| MapError::MappingFailed {
            reason: e.to_string(),
            source: e,
        })?;

        debug!("Memory Mapping Done");

        Ok(MappedDmaBuf {
            buf: self,
            len,
            mmap,
        })
    }
}

/// A `DmaBuf` mapped in memory
pub struct MappedDmaBuf {
    buf: DmaBuf,
    len: usize,
    mmap: MmapMut,
}

/// Error type to access a [`MappedDmaBuf`]
#[derive(Debug, thiserror::Error)]
pub enum BufferError {
    /// An Error occured while accessing the buffer file descriptor
    #[error("Could not access the buffer: {reason}")]
    FdAccess {
        /// Description of the Error
        reason: String,

        /// Source of the Error
        source: std::io::Error,
    },

    /// An Error occured in the closure
    #[error("The closure returned an error: {0}")]
    Closure(Box<dyn std::error::Error>),
}

impl MappedDmaBuf {
    /// Calls a closure to read the buffer content
    ///
    /// DMA-Buf requires the user-space to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. The closure will be run
    /// with those primitives called for a read access from the CPU.
    ///
    /// The result of the closure will be returned.
    ///
    /// # Errors
    ///
    /// Will return [Error] if the underlying ioctl or the closure fails
    pub fn read<A, F, R>(&self, f: F, arg: Option<A>) -> Result<R, BufferError>
    where
        F: Fn(&[u8], Option<A>) -> Result<R, Box<dyn std::error::Error>>,
    {
        let raw_fd = self.as_raw_fd();

        debug!("Preparing the buffer for read access");

        dma_buf_begin_cpu_read_access(raw_fd)?;

        debug!("Accessing the buffer");

        let ret = f(&self.mmap, arg)
            .map(|v| {
                debug!("Closure done without error");
                v
            })
            .map_err(|e| {
                debug!("Closure encountered an error {}", e);
                BufferError::Closure(e)
            });

        dma_buf_end_cpu_read_access(raw_fd)?;

        debug!("Buffer access done");

        ret
    }

    /// Calls a closure to read from and write to the buffer content
    ///
    /// DMA-Buf requires the user-space to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. The closure will be run
    /// with those primitives called for a read and write access from the CPU.
    ///
    /// The result of the closure will be returned on success. On failure, the closure must return
    /// `Error::Closure`
    ///
    /// # Errors
    ///
    /// Will return [Error] if the underlying ioctl or the closure fails
    pub fn readwrite<A, F, R>(&mut self, f: F, arg: Option<A>) -> Result<R, BufferError>
    where
        F: Fn(&mut [u8], Option<A>) -> Result<R, Box<dyn std::error::Error>>,
    {
        let raw_fd = self.as_raw_fd();

        debug!("Preparing the buffer for read/write access");

        dma_buf_begin_cpu_readwrite_access(raw_fd)?;

        debug!("Accessing the buffer");

        let ret = f(&mut self.mmap, arg)
            .map(|v| {
                debug!("Closure done without error");
                v
            })
            .map_err(|e| {
                debug!("Closure encountered an error {}", e);
                BufferError::Closure(e)
            });

        dma_buf_end_cpu_readwrite_access(raw_fd)?;

        debug!("Buffer access done");

        ret
    }

    /// Calls a closure to read from and write to the buffer content
    ///
    /// DMA-Buf requires the user-space to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. The closure will be run
    /// with those primitives called for a read and write access from the CPU.
    ///
    /// The closure must return () on success. On failure, the closure must return `Error::Closure`.
    ///
    /// # Errors
    ///
    /// Will return [Error] if the underlying ioctl or the closure fails
    pub fn write<A, F>(&mut self, f: F, arg: Option<A>) -> Result<(), BufferError>
    where
        F: Fn(&mut [u8], Option<A>) -> Result<(), Box<dyn std::error::Error>>,
    {
        let raw_fd = self.as_raw_fd();

        debug!("Preparing the buffer for write access");

        dma_buf_begin_cpu_write_access(raw_fd)?;

        debug!("Accessing the buffer");

        let ret = f(&mut self.mmap, arg)
            .map(|()| {
                debug!("Closure done without error");
            })
            .map_err(|e| {
                debug!("Closure encountered an error {}", e);
                BufferError::Closure(e)
            });

        dma_buf_end_cpu_write_access(raw_fd)?;

        debug!("Buffer access done");

        ret
    }
}

impl From<OwnedFd> for DmaBuf {
    fn from(owned: OwnedFd) -> Self {
        Self(owned)
    }
}

impl AsRawFd for DmaBuf {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsRawFd for MappedDmaBuf {
    fn as_raw_fd(&self) -> RawFd {
        self.buf.as_raw_fd()
    }
}

impl FromRawFd for DmaBuf {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        debug!("Importing DMABuf from File Descriptor {}", fd);

        Self(OwnedFd::from_raw_fd(fd))
    }
}

impl fmt::Debug for MappedDmaBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedDmaBuf")
            .field("DmaBuf", &self.buf)
            .field("len", &self.len)
            .field("address", &self.mmap.as_ptr())
            .finish()
    }
}
