// Copyright 2020-2021, Cerno
// Licensed under the MIT License
// See the LICENSE file or <http://opensource.org/licenses/MIT>
//
//! # DMA-Buf Helper Library
//!
//! The DMA-Buf mechanism in Linux is aimed at providing a way for the user-space to efficiently
//! share memory buffers between multiple devices, without any copy.
//!
//! This library provides a safe abstraction over this interface for Rust.

#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![deny(clippy::cargo)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

use std::{convert::TryInto, os::unix::io::RawFd, slice};

use ioctl::{
    dma_buf_begin_cpu_read_access, dma_buf_begin_cpu_readwrite_access,
    dma_buf_begin_cpu_write_access, dma_buf_end_cpu_read_access, dma_buf_end_cpu_readwrite_access,
    dma_buf_end_cpu_write_access,
};
use log::debug;
use mmap::{MapOption, MemoryMap};
use nix::sys::stat::fstat;

mod ioctl;

/// Error Type for dma-buf
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An Error occured in the closure
    #[error("Closure Error")]
    Closure,

    /// An Error happened when allocating a buffer
    #[error("System Error")]
    System(#[from] nix::Error),

    /// An Error occured when mapping the buffer
    #[error("mmap Error")]
    MMap(#[from] mmap::MapError),
}

/// A DMA-Buf buffer
#[derive(Debug)]
pub struct DmaBuf {
    fd: RawFd,
}

impl DmaBuf {
    /// Maps a `DmaBuf` for the CPU to access it
    ///
    /// # Errors
    ///
    /// Will return an error if either the Buffer's length can't be retrieved, or if the mmap call
    /// fails.
    pub fn memory_map(self) -> Result<MappedDmaBuf, Error> {
        debug!("Mapping DMA-Buf buffer with File Descriptor {}", self.fd);

        let stat = fstat(self.fd)?;
        let len = stat.st_size.try_into().unwrap();
        debug!("Valid buffer, size {}", len);

        let mmap = MemoryMap::new(
            len,
            &[
                MapOption::MapFd(self.fd),
                MapOption::MapOffset(0),
                MapOption::MapNonStandardFlags(libc::MAP_SHARED),
                MapOption::MapReadable,
                MapOption::MapWritable,
            ],
        )?;

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
    mmap: MemoryMap,
}

impl MappedDmaBuf {
    /// Calls a closure to read the buffer content
    ///
    /// DMA-Buf requires the user-space to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. The closure will be run
    /// with those primitives called for a read access from the CPU.
    ///
    /// The result of the closure will be returned on success. On failure, the closure must return
    /// `Error::Closure`
    ///
    /// # Errors
    ///
    /// Will return [Error] if the underlying ioctl or the closure fails
    pub fn read<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: Fn(&[u8]) -> Result<R, Error>,
    {
        let slice = unsafe { slice::from_raw_parts(self.mmap.data(), self.len) };

        debug!("Preparing the buffer for read access");

        dma_buf_begin_cpu_read_access(self.buf.fd)?;

        debug!("Accessing the buffer");

        let ret = f(slice);

        if ret.is_ok() {
            debug!("Closure done without error");
        } else {
            debug!("Closure encountered an error")
        }

        dma_buf_end_cpu_read_access(self.buf.fd)?;

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
    pub fn readwrite<A, F, R>(&self, f: F, arg: Option<A>) -> Result<R, Error>
    where
        F: Fn(&mut [u8], Option<A>) -> Result<R, Error>,
    {
        let slice = unsafe { slice::from_raw_parts_mut(self.mmap.data(), self.len) };

        debug!("Preparing the buffer for read/write access");

        dma_buf_begin_cpu_readwrite_access(self.buf.fd)?;

        debug!("Accessing the buffer");

        let ret = f(slice, arg);

        if ret.is_ok() {
            debug!("Closure done without error");
        } else {
            debug!("Closure encountered an error")
        }

        dma_buf_end_cpu_readwrite_access(self.buf.fd)?;

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
    pub fn write<A, F>(&self, f: F, arg: Option<A>) -> Result<(), Error>
    where
        F: Fn(&mut [u8], Option<A>) -> Result<(), Error>,
    {
        let slice = unsafe { slice::from_raw_parts_mut(self.mmap.data(), self.len) };

        debug!("Preparing the buffer for write access");

        dma_buf_begin_cpu_write_access(self.buf.fd)?;

        debug!("Accessing the buffer");

        let ret = f(slice, arg);

        if ret.is_ok() {
            debug!("Closure done without error");
        } else {
            debug!("Closure encountered an error")
        }

        dma_buf_end_cpu_write_access(self.buf.fd)?;

        debug!("Buffer access done");

        ret
    }
}

impl std::convert::TryFrom<RawFd> for DmaBuf {
    type Error = Error;

    fn try_from(fd: RawFd) -> Result<Self, Self::Error> {
        debug!("Importing DMABuf from File Descriptor {}", fd);
        Ok(Self { fd })
    }
}

impl std::os::unix::io::AsRawFd for DmaBuf {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl std::os::unix::io::AsRawFd for MappedDmaBuf {
    fn as_raw_fd(&self) -> RawFd {
        self.buf.fd
    }
}

impl std::fmt::Debug for MappedDmaBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedDmaBuf")
            .field("DmaBuf", &self.buf)
            .field("len", &self.len)
            .field("address", &self.mmap.data())
            .finish()
    }
}

impl Drop for DmaBuf {
    fn drop(&mut self) {
        debug!("Closing buffer {}", self.fd);
        nix::unistd::close(self.fd).unwrap();
    }
}
