use std::ops::{Not, Range, Deref, DerefMut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard as ReadGuard, TryLockError};
use std::time::Duration;
use std::{cmp, ptr, thread};

use align::AlignedBufs;

mod align;

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
    bufs: AlignedBufs<T>,
    lock: RwLock<WriteBuf>,
    waiting: AtomicIsize,
    threads: usize,
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
                bufs: bufs,
                lock: RwLock::new(WriteBuf::A),
                waiting: AtomicIsize::new(0),
                threads: threads,
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
            parent: self,
            guard: guard,
            buf: buf,
        })
    }

    pub fn read(&self) -> ReadHandle<T> {
        let guard = self.lock.read().unwrap();
        let buf = unsafe { self.get_buf(!*guard) };

        ReadHandle {
            parent: self,
            guard: guard,
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

    unsafe fn get_buf(&self, write_buf: WriteBuf) -> &mut [T] {
        match write_buf {
            WriteBuf::A => self.bufs.bufa(),
            WriteBuf::B => self.bufs.bufb(),
        }
    }

    pub fn waiting(&self) -> bool {
        self.waiting.load(Ordering::Acquire) != 0
    }

    fn swap(&self) -> bool {        
        let mut waiting = self.waiting.load(Ordering::Acquire);

        while { 
            waiting = self.waiting.load(Ordering::Acquire);
            waiting > 0
        } {
            thread::sleep(Duration::new(0, 0));            
        }

        true        
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
            self.bufs.drop_all();
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

fn next_multiple(from: usize, base: usize) -> usize {
    (from / base + if from % base != 0 { 1 } else { 0 })
        * base
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
            let chunk_size = next_multiple(raw_chunk_size, self.aligned_len);

            let end = cmp::min(start + chunk_size, self.end);
            self.curr = end;
            self.rem_count -= 1;

            Some(start .. end)
        } else {
            None
        } 
    }
}

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
    parent: &'a DblBuf<T>,
    guard: ReadGuard<'a, WriteBuf>,
    buf: &'a [T],
}

impl<'a, T: 'a> Deref for ReadHandle<'a, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.buf
    }
}

fn gcf_3(x: usize, y: usize, z: usize) -> usize {
    gcf(x, gcf(y, z))
}

fn lcm(x: usize, y: usize) -> usize {
    x * y / gcf(x, y)
}

fn gcf(x: usize, y: usize) -> usize {
    let (mut num, mut denom) = if x > y { (x, y) } else { (y, x) };

    let mut rem;

    while {rem = num % denom; rem != 0} {
        num = denom;
        denom = rem;
    }

    denom
}

