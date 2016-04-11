#![feature(alloc, heap_api)]

extern crate alloc;

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Not, Range, Deref, DerefMut};
use std::sync::{
    Arc, RwLock, TryLockError,
    RwLockReadGuard as ReadGuard,
    RwLockWriteGuard as WriteGuard, 
};
use std::{cmp, ptr};

use align::AlignedBufs;

mod align;
mod math;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WriteBuf {
    A,
    B,
}

impl Not for WriteBuf {
    type Output = Self;

    fn not(self) -> Self { 
        match self { 
            WriteBuf::A => WriteBuf::B,
            WriteBuf::B => WriteBuf::A,
        }
    }
}

struct BufState<T> {
    ranges: Vec<(usize, usize)>,
    write_buf: WriteBuf,
    _marker: PhantomData<*const T>,
}

impl<T> BufState<T> {
    fn new(threads: usize, len: usize) -> Self {
        let ranges = ChunkRanges::new::<T>(threads, len);

        BufState {
            ranges: ranges.collect(),
            write_buf: WriteBuf::A,
            _marker: PhantomData,
        }
    }

    fn range(&self, thread: usize) -> Range<usize> {
        let (start, end) = self.ranges[thread];
        start .. end
    }

    fn redistribute(&mut self, new_len: usize) {
        let threads = self.ranges.len();
        let ranges = ChunkRanges::new::<T>(threads, new_len);

        self.ranges.truncate(0);
        self.ranges.extend(ranges);
    }
}

struct ChunkRanges {
    aligned_len: usize,
    rem_count: usize,
    curr: usize,
    end: usize,
}

impl ChunkRanges {
    fn new<T>(threads: usize, len: usize) -> Self {
        let aligned_len = align::cache_aligned_len::<T>();

        ChunkRanges {
            aligned_len: aligned_len,
            rem_count: threads,
            curr: 0,
            end: len,
        }
    }
}

impl Iterator for ChunkRanges {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.rem_count > 0 {
            let start = self.curr;
            let rem_len = self.end - self.curr;
        
            let raw_chunk_size = rem_len / self.rem_count;

            // Get the next multiple of `self.aligned_len` up from `raw_chunk_size`
            let chunk_size = math::next_multiple(raw_chunk_size, self.aligned_len);

            let end = cmp::min(start + chunk_size, self.end);
            self.curr = end;
            self.rem_count -= 1;

            Some((start, end))
        } else {
            None
        } 
    }
}

pub type NewDblBuf<T> = (Arc<DblBuf<T>>, Handles<T>);

pub struct DblBuf<T> {
    bufs: UnsafeCell<AlignedBufs<T>>,
    lock: RwLock<BufState<T>>, 
}

impl<T> DblBuf<T> {
    pub fn with_cap(threads: usize, cap: usize) -> NewDblBuf<T> {
        let bufs = AlignedBufs::alloc(cap);
        let state = BufState::new(threads, 0);

        Self::new(bufs, state, threads)                        
    } 

    pub fn with_bufs(threads: usize, mut bufa: Vec<T>, mut bufb: Vec<T>) -> NewDblBuf<T> {
        assert_eq!(bufa.len(), bufb.len());

        let len = bufa.len();
        let mut bufs = AlignedBufs::alloc(len);

        unsafe { 
            ptr::copy_nonoverlapping(bufa.as_ptr(), bufs.bufa_ptr(), len);
            bufa.set_len(0);
            ptr::copy_nonoverlapping(bufb.as_ptr(), bufs.bufb_ptr(), len);
            bufb.set_len(0);

            bufs.set_len(len);
        }   

        let state = BufState::new(threads, len);
           
        Self::new(bufs, state, threads)                        
    }

    fn new(bufs: AlignedBufs<T>, state: BufState<T>, threads: usize) -> NewDblBuf<T> {
        assert_eq!(state.ranges.len(), threads);

        let dblbuf = Arc::new(DblBuf {
            bufs: UnsafeCell::new(bufs),
            lock: RwLock::new(state),
        });
            
        (
            dblbuf.clone(),
            Handles {
                buf: dblbuf,
                threads: 0 .. threads 
            }
        )
    }

    pub fn try_read(&self) -> Option<ReadHandle<T>> {
        let guard = match self.lock.try_read() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(err)) => Err(err).unwrap(),
        };

        let buf = unsafe { self.get_buf(!guard.write_buf) };
        
        Some(ReadHandle {
            _parent: self,
            _guard: guard,
            buf: buf,
        })
    }

    pub fn read(&self) -> ReadHandle<T> {
        let guard = self.lock.read().unwrap();
        let buf = unsafe { self.get_buf(!guard.write_buf) };

        ReadHandle {
            _parent: self,
            _guard: guard,
            buf: buf,
        }
    }

    unsafe fn write(&self, thread: usize) -> WriteHandle<T> {
        let guard = self.lock.read().unwrap();
        let range = guard.range(thread);

        let buf = self.get_buf(guard.write_buf);

        WriteHandle {
            parent: self,
            guard: guard,
            buf: &mut buf[range],
        }
    }

    unsafe fn get_bufs(&self, write_buf: WriteBuf) -> (&mut [T], &mut [T]) {
        (
            self.get_buf(write_buf),
            self.get_buf(!write_buf),
        )
    }

    unsafe fn get_buf(&self, write_buf: WriteBuf) -> &mut [T] {
        let bufs = self.bufs();

        match write_buf {
            WriteBuf::A => bufs.bufa(),
            WriteBuf::B => bufs.bufb(),
        }
    } 


    unsafe fn bufs_mut(&self) -> &mut AlignedBufs<T> {
        &mut *self.bufs.get()
    }

    unsafe fn bufs(&self) -> &AlignedBufs<T> {
        &*self.bufs.get()
    }

    pub fn swap(&self) {
        let curr = match self.lock.try_read() {
            Ok(guard) => guard.write_buf,
            // Already trying to swap, just wait for release
            Err(TryLockError::WouldBlock) => {
                let _ = self.lock.read().unwrap();
                return;
            },
            // Lock poisoned, panic
            e => { let _ = e.unwrap(); unreachable!() },
        };

        self.lock.write().unwrap().write_buf = !curr;

        println!("Swap! {:?} -> {:?}", curr, !curr);
    }

    /// Block until exclusive access to both buffers is available (preempt a swap).
    pub fn exclusive(&self) -> Exclusive<T> {
        let guard = self.lock.write().unwrap();

        Exclusive {
            parent: self,
            guard: guard,
        }
    }
}

impl<T: Clone> DblBuf<T> {
    pub fn new_with_elem(threads: usize, buf_size: usize, elem: T) -> (Arc<Self>, Handles<T>) {
        let bufa = vec![elem.clone(); buf_size];
        let bufb = vec![elem; buf_size];
        Self::with_bufs(threads, bufa, bufb)
    }
}

pub struct BufHandle<T> { 
    buf: Arc<DblBuf<T>>,
    thread: usize,
}

impl<T> BufHandle<T> {
    pub fn read_write(&mut self) -> (ReadHandle<T>, WriteHandle<T>) {
        (
            self.buf.read(),
            unsafe { self.buf.write(self.thread) },
        )
    }

    pub fn swap(&self) {
        self.buf.swap();        
    }
}


/// An iterator which yields `BufHandle`.
pub struct Handles<T> {
    buf: Arc<DblBuf<T>>,
    threads: Range<usize>,    
}    

impl<T> Iterator for Handles<T> {
    type Item = BufHandle<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.threads.next().map(|thread| 
            BufHandle {
                buf: self.buf.clone(),
                thread: thread,
            }
        )
    }
}

pub struct WriteHandle<'a, T: 'a> {
    parent: &'a DblBuf<T>,
    guard: ReadGuard<'a, BufState<T>>,
    buf: &'a mut [T],
}

impl<'a, T: 'a> DerefMut for WriteHandle<'a, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.buf
    }
}

impl<'a, T: 'a> Deref for WriteHandle<'a, T> {
    type Target = [T];
    
    fn deref(&self) -> &[T] {
        self.buf
    }
}

/// Get the indices in the original buffer for the given `WriteHandle`.
pub fn write_range<T>(hndl: &WriteHandle<T>) -> (usize, usize) {
    let ptr = hndl.buf.as_ptr();
    let orig_ptr = unsafe { hndl.parent.get_buf(hndl.guard.write_buf).as_ptr() };
    let offset = orig_ptr as usize - ptr as usize;

    (offset, offset + hndl.buf.len())
}

pub struct ReadHandle<'a, T: 'a> {
    _parent: &'a DblBuf<T>,
    _guard: ReadGuard<'a, BufState<T>>,
    buf: &'a [T],
}

impl<'a, T: 'a> Deref for ReadHandle<'a, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.buf
    }
}

pub struct Exclusive<'a, T: 'a> {
    parent: &'a DblBuf<T>,
    guard: WriteGuard<'a, BufState<T>>,
}

impl<'a, T: 'a> Exclusive<'a, T> {
    pub fn bufs(&mut self) -> (&mut [T], &mut [T]) {
        // Safe because we have exclusive access to the buffers
        unsafe { self.parent.get_bufs(self.guard.write_buf) }
    }

    pub fn resize_with_fn<F: FnMut(usize, bool) -> T>(&mut self, new_len: usize, resize_fn: F) {
        unsafe {
            self.parent.bufs_mut().resize_with_fn(new_len, resize_fn);
        }
    }

    pub fn swap(&mut self) {
        let curr = self.guard.write_buf;
        self.guard.write_buf = !curr;
    }
}
