# DMA-Buf Helper Library

The DMA-Buf mechanism in Linux is aimed at providing a way for the user-space to efficiently
share memory buffers between multiple devices, without any copy.

This library provides a safe abstraction over this interface for Rust.

## Basic usage

```
use dma_buf::DmaBuf;
use dma_buf::test;
let buf: &DmaBuf = test::get_dma_buf();

{
    // Request sync and create an access guard.
    // Multiple read-only accesses can co-exist
    let mmap = buf.memory_map_ro().unwrap();
    // The actual slice
    let data = mmap.as_slice();
    if data.len() >= 4 {
        println!("Data buffer: {:?}...", &data[..4]);
    }
} // `mmap` goes out of scope and unmaps the buffer

let buf: &mut DmaBuf = test::get_dma_buf_mut();

{
    // Write access is only allowed for mutable borrows
    let mut mmap_rw = buf.memory_map_rw().unwrap();
    let data = mmap_rw.as_slice_mut();
    if data.len() >= 4 {
        data[0] = 0;
        println!("Data buffer: {:?}...", &data[..4]);
    }
}
```