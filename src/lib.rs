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
#[non_exhaustive]
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
    fn memory_map(&self) -> Result<(usize, *mut u8), MapError> {
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
        
        Ok((len, mapping_ptr))
    }

    /// Maps a `DmaBuf` for the CPU to access it read-
    ///
    /// DMA-Buf [requires the user-space](https://docs.kernel.org/driver-api/dma-buf.html#cpu-access-to-dma-buffer-objects) to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. 
    ///
    /// # Panics
    ///
    /// If the buffer size reported by the kernel (`i64`) cannot fit into an `usize`.
    ///
    /// # Errors
    ///
    /// Will return an error if either the Buffer's length can't be retrieved, or if the mmap call
    /// fails.
    pub fn memory_map_ro(&self) -> Result<MappedDmaBufRo<'_>, MapError> {
        let (len, mapping_ptr) = self.memory_map()?;
        MappedDmaBufRo::new(self, len, mapping_ptr)
    }

    /// Maps a `DmaBuf` for the CPU to access it read-write
    ///
    /// DMA-Buf [requires the user-space](https://docs.kernel.org/driver-api/dma-buf.html#cpu-access-to-dma-buffer-objects) to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. 
    ///
    /// # Panics
    ///
    /// If the buffer size reported by the kernel (`i64`) cannot fit into an `usize`.
    ///
    /// # Errors
    ///
    /// Will return an error if either the Buffer's length can't be retrieved, or if the mmap call
    /// fails.
    pub fn memory_map_rw(&mut self) -> Result<MappedDmaBufRw<'_>, MapError> {
        let (len, mapping_ptr) = self.memory_map()?;
        MappedDmaBufRw::new(self, len, mapping_ptr)
    }
    
    /// Maps a `DmaBuf` for the CPU to access it write-only
    ///
    /// DMA-Buf [requires the user-space](https://docs.kernel.org/driver-api/dma-buf.html#cpu-access-to-dma-buffer-objects) to call the `DMA_BUF_IOCTL_SYNC` ioctl before and after any
    /// CPU access to a buffer in order to maintain the cache coherency. 
    ///
    /// # Panics
    ///
    /// If the buffer size reported by the kernel (`i64`) cannot fit into an `usize`.
    ///
    /// # Errors
    ///
    /// Will return an error if either the Buffer's length can't be retrieved, or if the mmap call
    /// fails.
    pub fn memory_map_wo(&mut self) -> Result<MappedDmaBufWo<'_>, MapError> {
        let (len, mapping_ptr) = self.memory_map()?;
        MappedDmaBufWo::new(self, len, mapping_ptr)
    }
}

/// A `DmaBuf` mapped in memory.
///
/// This uses an arbitrary T, even though the only 2 types possible are &DmaBuf and &mut DmaBuf, but Rust doesn't provide a generic over mutability.
///
/// The mutability makes a difference with &DmaBuf: that one can be mapped multiple times in independent places.
struct MappedDmaBuf<T> {
    buf: T,
    len: usize,
    mmap: *mut u8,
}

/// A read-only Dmabuf mapped in memory. The underlying data gets cache-synced on creation and destruction.
#[derive(Debug)]
pub struct MappedDmaBufRo<'a>(MappedDmaBuf<&'a DmaBuf>);

/// A read-write Dmabuf mapped in memory. The underlying data gets cache-synced on creation and destruction.
#[derive(Debug)]
pub struct MappedDmaBufRw<'a>(MappedDmaBuf<&'a mut DmaBuf>);

/// A write-only Dmabuf mapped in memory. The underlying data gets cache-synced on creation and destruction.
#[derive(Debug)]
pub struct MappedDmaBufWo<'a>(MappedDmaBuf<&'a mut DmaBuf>);


impl<T: AsFd> MappedDmaBuf<T> {
    fn as_slice(&self) -> &[u8] {
        // SAFETY: We know that the pointer is valid, and the buffer length is at least equal to
        // self.len bytes. The backing buffer won't be mutated by the kernel, our structure is the
        // sole owner of the pointer, and it won't be mutated in our code either, so we're safe.
        unsafe { slice::from_raw_parts(self.mmap, self.len) }
    }
}

impl MappedDmaBuf<&'_ mut DmaBuf> {
    fn as_slice_mut(&mut self) -> &mut [u8] {
        // SAFETY: We know that the pointer is valid, and the buffer length is at least equal to
        // self.len bytes. The backing buffer won't be mutated by the kernel, our structure is the
        // sole owner of the pointer, and it won't be mutated in our code either, so we're safe.
        unsafe { slice::from_raw_parts_mut(self.mmap, self.len) }
    }
}

impl<'a> MappedDmaBufRo<'a> {
    fn new(buf: &'a DmaBuf, len: usize, mapping_ptr: *mut u8) -> Result<Self, MapError> {
        debug!("Preparing the buffer for read access");

        dma_buf_begin_cpu_read_access(buf.as_fd())?;

        Ok(Self(MappedDmaBuf {
            buf,
            len,
            mmap: mapping_ptr,
        }))
    }
    
    /// Access the underlying data directly
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

// maybe TODO: implement a manual .release(self) -> Result<(), Self> which passes the failure to the caller.
impl Drop for MappedDmaBufRo<'_> {
    fn drop(&mut self) {
        if let Err(e) = dma_buf_end_cpu_read_access(self.0.buf.as_fd()) {
            warn!("End read access failed: {}", e);
        }

        debug!("Buffer access done");
    }
}

impl<'a> MappedDmaBufRw<'a> {
    fn new(buf: &'a mut DmaBuf, len: usize, mapping_ptr: *mut u8) -> Result<Self, MapError> {
        debug!("Preparing the buffer for read/write access");

        dma_buf_begin_cpu_readwrite_access(buf.as_fd())?;

        Ok(Self(MappedDmaBuf {
            buf,
            len,
            mmap: mapping_ptr,
        }))
    }
    
    /// Access the underlying data directly
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
    
    /// Access the underlying data mutably
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        self.0.as_slice_mut()
    }
}

impl Drop for MappedDmaBufRw<'_> {
    fn drop(&mut self) {
        if let Err(e) = dma_buf_end_cpu_readwrite_access(self.0.buf.as_fd()) {
            warn!("End read|write access failed: {}", e);
        }

        debug!("Buffer access done");
    }
}


impl<'a> MappedDmaBufWo<'a> {
    fn new(buf: &'a mut DmaBuf, len: usize, mapping_ptr: *mut u8) -> Result<Self, MapError> {
        debug!("Preparing the buffer for write access");

        dma_buf_begin_cpu_write_access(buf.as_fd())?;

        Ok(Self(MappedDmaBuf {
            buf,
            len,
            mmap: mapping_ptr,
        }))
    }
    
    /// Access the underlying data mutably.
    ///
    /// Data in this struct is cache-coherently guarded only for write access, so reading may give unexpected results.
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        self.0.as_slice_mut()
    }
}

// maybe TODO: implement a manual .release(self) -> Result<(), Self> which passes the failure to the caller.
impl Drop for MappedDmaBufWo<'_> {
    fn drop(&mut self) {
        if let Err(e) = dma_buf_end_cpu_write_access(self.0.buf.as_fd()) {
            warn!("End write access failed: {}", e);
        }

        debug!("Buffer access done");
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

impl FromRawFd for DmaBuf {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        debug!("Importing DMABuf from File Descriptor {}", fd);

        // SAFETY: We're just forwarding the FromRawFd implementation to our inner OwnerFd type.
        // We're having exactly the same safety guarantees.
        Self(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

impl<T: fmt::Debug> fmt::Debug for MappedDmaBuf<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedDmaBuf")
            .field("DmaBuf", &self.buf)
            .field("len", &self.len)
            .field("address", &self.mmap)
            .finish()
    }
}

impl<T> Drop for MappedDmaBuf<T> {
    fn drop(&mut self) {
        debug!("Unmapping");
        // SAFETY: It's not clear what rustix expects from a safety perspective, but our pointer is
        // valid, and is a void pointer at least.
        if unsafe { munmap(self.mmap.cast::<c_void>(), self.len) }.is_err() {
            warn!("unmap failed!");
        }
    }
}
