use std::cell::UnsafeCell;
use std::ops::{Not, Range, Deref, DerefMut};
use std::sync::{
    Arc, RwLock, TryLockError,
    RwLockReadGuard as ReadGuard,
    RwLockWriteGuard as WriteGuard, 
};
use std::{cmp, ptr};

use align::Bufs;

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

pub struct DblBuf<T> {
    bufs: UnsafeCell<AlignedBufs<T>>,
    lock: RwLock<WriteBuf>, 
}

impl<T> DblBuf<T> {
    pub fn new(threads: usize, mut bufa: Vec<T>, mut bufb: Vec<T>) -> (Arc<Self>, Handles<T>) {
        assert_eq!(bufa.len(), bufb.len());

        let bufs = AlignedBufs::alloc(bufa.len());

        unsafe { 
            ptr::copy_nonoverlapping(bufa.as_ptr(), bufs.bufa().as_mut_ptr(), bufa.len());
            bufa.set_len(0);
            ptr::copy_nonoverlapping(bufb.as_ptr(), bufs.bufb().as_mut_ptr(), bufb.len());
            bufb.set_len(0);
        }   

        let len = bufs.len();
        let aligned_len = align::cache_aligned_len::<T>();
            
        let dblbuf = Arc::new(
            DblBuf {
                bufs: UnsafeCell::new(bufs),
                lock: RwLock::new(WriteBuf::A),
            }
        );

        (
            dblbuf.clone(),
            Handles {
                buf: dblbuf,
                ranges: ChunkRanges {
                    aligned_len: aligned_len,
                    rem_count: threads,
                    curr: 0,
                    end: len,
                }
            }
        )                
    }

    pub fn try_read(&self) -> Option<ReadHandle<T>> {
        let guard = match self.lock.try_read() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(err)) => Err(err).unwrap(),
        };

        let buf = unsafe { self.get_buf(!*guard) };
        
        Some(ReadHandle {
            _parent: self,
            _guard: guard,
            buf: buf,
        })
    }

    pub fn read(&self) -> ReadHandle<T> {
        let guard = self.lock.read().unwrap();
        let buf = unsafe { self.get_buf(!*guard) };

        ReadHandle {
            _parent: self,
            _guard: guard,
            buf: buf,
        }
    }

    unsafe fn write(&self, range: Range<usize>) -> WriteHandle<T> {
        let guard = self.lock.read().unwrap();

        let buf = self.get_buf(*guard);

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
            Ok(guard) => *guard,
            // Already trying to swap, just wait for release
            Err(TryLockError::WouldBlock) => {
                let _ = self.lock.read().unwrap();
                return;
            },
            // Lock poisoned, panic
            e => { let _ = e.unwrap(); unreachable!() },
        };

        *self.lock.write().unwrap() = !curr;

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
        Self::new(threads, bufa, bufb)
    }
}

impl<T> Drop for DblBuf<T> {
    fn drop(&mut self) {
        // Safe because there should be no references to this struct anymore.
        unsafe {
            let mut bufs = self.bufs_mut();
            bufs.drop_all();
            bufs.free();
        }
    }
}

pub struct BufHandle<T> { 
    buf: Arc<DblBuf<T>>,
    mut_range: Range<usize>,
}

impl<T> BufHandle<T> {
    pub fn read_write(&mut self) -> (ReadHandle<T>, WriteHandle<T>) {
        (
            self.buf.read(),
            unsafe { self.buf.write(self.mut_range.clone()) },
        )
    }

    pub fn swap(&self) {
        self.buf.swap();        
    }
}

struct ChunkRanges {
    aligned_len: usize,
    rem_count: usize,
    curr: usize,
    end: usize,
}

impl Iterator for ChunkRanges {
    type Item = Range<usize>;

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

            Some(start .. end)
        } else {
            None
        } 
    }
}

/// An iterator which yields `BufHandle`.
pub struct Handles<T> {
    buf: Arc<DblBuf<T>>,
    ranges: ChunkRanges,    
}    

impl<T> Iterator for Handles<T> {
    type Item = BufHandle<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.ranges.next().map(|range| 
            BufHandle {
                buf: self.buf.clone(),
                mut_range: range,
            }
        )
    }
}

pub struct WriteHandle<'a, T: 'a> {
    parent: &'a DblBuf<T>,
    guard: ReadGuard<'a, WriteBuf>,
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
    let orig_ptr = unsafe { hndl.parent.get_buf(*hndl.guard).as_ptr() };
    let offset = orig_ptr as usize - ptr as usize;

    (offset, offset + hndl.buf.len())
}

pub struct ReadHandle<'a, T: 'a> {
    _parent: &'a DblBuf<T>,
    _guard: ReadGuard<'a, WriteBuf>,
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
    guard: WriteGuard<'a, WriteBuf>,
}

impl<'a, T: 'a> Exclusive<'a, T> {
    pub fn bufs(&mut self) -> (&mut [T], &mut [T]) {
        // Safe because we have exclusive access to the buffers
        unsafe { self.parent.get_bufs(*self.guard) }
    }

    pub fn resize_with_fn<F: FnMut(usize) -> T>(&mut self, new_len: usize, mut resize_fn: F) {
        let bufs = unsafe { self.parent.bufs_mut() };

        let old_len = bufs.len();

        unsafe {
            if new_len < bufs.len() {
                bufs.drop_range(old_len, new_len);
            }
            bufs.realloc(new_len);
        }
        
        if new_len > old_len {
            let mut make_buf = || (old_len .. new_len).map(&mut resize_fn).collect::<Vec<T>>();
            let mut bufa_extend = make_buf();
            let mut bufb_extend = make_buf();

            unsafe {
                let mut bufa = &mut bufs.bufa()[new_len ..];

                ptr::copy_nonoverlapping(
                    bufa_extend.as_ptr(),
                    bufa.as_mut_ptr(),
                    bufa.len(),
                );

                bufa_extend.set_len(0);

                let mut bufb = &mut bufs.bufb()[new_len ..];

                ptr::copy_nonoverlapping(
                    bufb_extend.as_ptr(),
                    bufb.as_mut_ptr(),
                    bufb.len(),
                );

                bufb_extend.set_len(0);
            }
        }
    }

    pub fn swap(&mut self) {
        let curr = *self.guard;
        *self.guard = !curr;
    }
}
