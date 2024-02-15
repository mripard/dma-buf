// Copyright 2020-2021, Cerno
// Licensed under the MIT License
// See the LICENSE file or <http://opensource.org/licenses/MIT>

#![cfg_attr(
    feature = "nightly",
    feature(
        type_privacy_lints,
        non_exhaustive_omitted_patterns_lint,
        strict_provenance
    )
)]
#![cfg_attr(
    feature = "nightly",
    warn(
        fuzzy_provenance_casts,
        lossy_provenance_casts,
        unnameable_types,
        non_exhaustive_omitted_patterns,
        clippy::empty_enum_variants_with_brackets
    )
)]
#![doc = include_str!("../README.md")]

use core::{ffi::c_void, fmt, num::TryFromIntError, ptr, slice};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

use log::{debug, warn};
use rustix::{
    fs::fstat,
    mm::{mmap, munmap, MapFlags, ProtFlags},
    param::page_size,
};

mod ioctl;
use ioctl::{
    dma_buf_begin_cpu_read_access, dma_buf_begin_cpu_readwrite_access,
    dma_buf_begin_cpu_write_access, dma_buf_end_cpu_read_access, dma_buf_end_cpu_readwrite_access,
    dma_buf_end_cpu_write_access,
};

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

    /// An Error occurred while converting between Integer types
    #[error("Integer Conversion Error")]
    IntegerConversionFailed(#[from] TryFromIntError),
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
        debug!("Mapping DMA-Buf buffer with File Descriptor {:#?}", self.0);

        let stat = fstat(&self.0).map_err(|e| MapError::FdAccess {
            reason: e.to_string(),
            source: std::io::Error::from(e),
        })?;

        let len = usize::try_from(stat.st_size)?.next_multiple_of(page_size());
        debug!("Valid buffer, size {len}");

        // SAFETY: It's unclear at this point what the exact safety requirements from mmap are, but
        // our fd is valid and the length is aligned, so that's something.
        let mapping_ptr = unsafe {
            mmap(
                ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                &self.0,
                0,
            )
        }
        .map(<*mut c_void>::cast::<u8>)
        .map_err(|e| MapError::MappingFailed {
            reason: e.to_string(),
            source: std::io::Error::from(e),
        })?;

        debug!("Memory Mapping Done");

        Ok(MappedDmaBuf {
            buf: self,
            len,
            mmap: mapping_ptr,
        })
    }
}

/// A `DmaBuf` mapped in memory
pub struct MappedDmaBuf {
    buf: DmaBuf,
    len: usize,
    mmap: *mut u8,
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
    fn as_slice(&self) -> &[u8] {
        // SAFETY: We know that the pointer is valid, and the buffer length is at least equal to
        // self.len bytes. The backing buffer won't be mutated by the kernel, our structure is the
        // sole owner of the pointer, and it won't be mutated in our code either, so we're safe.
        unsafe { slice::from_raw_parts(self.mmap, self.len) }
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        // SAFETY: We know that the pointer is valid, and the buffer length is at least equal to
        // self.len bytes. The backing buffer won't be mutated by the kernel, our structure is the
        // sole owner of the pointer, and it won't be mutated in our code either, so we're safe.
        unsafe { slice::from_raw_parts_mut(self.mmap, self.len) }
    }

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
        debug!("Preparing the buffer for read access");

        dma_buf_begin_cpu_read_access(self.buf.as_fd())?;

        debug!("Accessing the buffer");

        let ret = {
            let bytes = self.as_slice();

            f(bytes, arg)
                .map(|v| {
                    debug!("Closure done without error");
                    v
                })
                .map_err(|e| {
                    debug!("Closure encountered an error {}", e);
                    BufferError::Closure(e)
                })
        };

        dma_buf_end_cpu_read_access(self.buf.as_fd())?;

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
        debug!("Preparing the buffer for read/write access");

        dma_buf_begin_cpu_readwrite_access(self.buf.as_fd())?;

        debug!("Accessing the buffer");

        let ret = {
            let bytes = self.as_slice_mut();

            f(bytes, arg)
                .map(|v| {
                    debug!("Closure done without error");
                    v
                })
                .map_err(|e| {
                    debug!("Closure encountered an error {}", e);
                    BufferError::Closure(e)
                })
        };

        dma_buf_end_cpu_readwrite_access(self.buf.as_fd())?;

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
        debug!("Preparing the buffer for write access");

        dma_buf_begin_cpu_write_access(self.buf.as_fd())?;

        debug!("Accessing the buffer");

        let ret = {
            let bytes = self.as_slice_mut();

            f(bytes, arg)
                .map(|()| {
                    debug!("Closure done without error");
                })
                .map_err(|e| {
                    debug!("Closure encountered an error {}", e);
                    BufferError::Closure(e)
                })
        };

        dma_buf_end_cpu_write_access(self.buf.as_fd())?;

        debug!("Buffer access done");

        ret
    }
}

impl From<OwnedFd> for DmaBuf {
    fn from(owned: OwnedFd) -> Self {
        Self(owned)
    }
}

impl AsFd for DmaBuf {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for DmaBuf {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsFd for MappedDmaBuf {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.buf.as_fd()
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

        // SAFETY: We're just forwarding the FromRawFd implementation to our inner OwnerFd type.
        // We're having exactly the same safety guarantees.
        Self(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

impl fmt::Debug for MappedDmaBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedDmaBuf")
            .field("DmaBuf", &self.buf)
            .field("len", &self.len)
            .field("address", &self.mmap)
            .finish()
    }
}

impl Drop for MappedDmaBuf {
    fn drop(&mut self) {
        // SAFETY: It's not clear what rustix expects from a safety perspective, but our pointer is
        // valid, and is a void pointer at least.
        if unsafe { munmap(self.mmap.cast::<c_void>(), self.len) }.is_err() {
            warn!("unmap failed!");
        }
    }
}
