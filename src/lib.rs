
use std::cell::UnsafeCell;
use std::iter::RangeTo;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard as ReadGuard, TryLockError};
use std::{ops, ptr};

use align::AlignedBufs;

mod align;

#[derive(Copy, Debug, PartialEq, Eq)]
enum WriteBuf {
    A,
    B,
}

impl ops::Not for WriteBuf {
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
    swap_waiting: AtomicBool,
}

impl<T> DblBuf<T> {
    pub fn new(threads: usize, mut bufa: Vec<T>, mut bufb: Vec<T>) -> Handles<T> {
        assert_eq!(bufa.len(), bufb.len());

        let bufs = AlignedBufs::alloc(bufa.len());

        unsafe { 
            ptr::copy_nonoverlapping(bufa.as_ptr(), bufs.bufa().as_mut_ptr(), bufa.len());
            bufa.set_len(0);
            ptr::copy_nonoverlapping(bufb.as_ptr(), bufs.bufb().as_mut_ptr(), bufb.len());
            bufb.set_len(0);
        }       
    }

    pub fn try_read(&self) -> Option<ReadHandle> {
        let guard = match self.lock.try_read() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(err)) => Err(err).unwrap(),
        };

        let buf = unsafe { self.get_buf(!*guard) };
        
        ReadHandle {
            guard: guard,
            buf: buf,
        }
    }

    pub fn read(&self) -> ReadHandle {
        let guard = self.lock.read().unwrap();
        let buf = unsafe { self.get_buf(!*guard) };

        ReadHandle {
            guard: guard,
            buf: buf,
        }
    }

    unsafe fn write(&self, range: Range<usize>) -> WriteHandle {
        let guard = self.lock.read().unwrap();

        let buf = self.get_buf(*guard);

        WriteHandle {
            guard: guard,
            buf: &mut buf[range],
        }
    }

    unsafe fn get_buf(&self, write_buf: WriteBuf) -> &mut [T] {
        match *write_buf {
            WriteBuf::A => self.bufs.bufa(),
            WriteBuf::B => self.bufs.bufb(),
        }
    }

    pub fn swap_waiting(&self) -> bool {
        self.swap_waiting.load(Ordering::Acquire)
    }

    fn swap(&self) {
        let swap_waiting = self.swap_waiting.swap(true, Ordering::AcqRel);

        if !swap_waiting { 
            let write = self.lock.write().unwrap();

            *write = !*write;

            swap_waiting.store(false, Ordering::Release);
            return;
        }

        let mut last = ;

        while {last = ; 



    }
}

impl<T: Clone> DblBuf<T> {
    pub fn new_with_elem<T: Clone>(threads: usize, buf_size: usize, elem: T) -> Handles<T> {
        let bufa = vec![elem.clone(); buf_size];
        let bufb = vec![elem; buf_size];
        new(threads, bufa, bufb);
    }
}

#[derive(Debug)]
pub struct BufHandle<T> { 
    buf: Arc<DblBuf<T>>,
    mut_range: Range<usize>,
}

impl<T> BufHandle<T> {
    pub fn read_write(&mut self) -> (ReadHandle, WriteHandle) {
        (
            self.buf.read(),
            unsafe { self.buf.write() },
        )
    }

    pub fn swap(&self) {
        self.buf.swap();        
    }
}

impl !Sync for BufHandle {}

pub struct Handles<T> {
    buf: Arc<DblBuf<T>>,
    

impl Iterator for Handles<T> {
    type Item = BufHandle<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ThreadToken)
    }
}

pub struct WriteHandle<'a, T> {
    guard: ReadGuard<'a, T>,
    buf: &'a mut [T],
}

impl DerefMut for WriteHandle<'a, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.buf
    }
}

impl Deref for WriteHandle<'a, T> {
    type Target = [T];
    
    fn deref(&self) -> &[T] {
        self.buf
    }
}

pub struct ReadHandle<'a, T> {
    guard: ReadGuard<'a, T>,
    buf: &'a [T],
}

impl Deref for ReadHandle<[T]> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.buf
    }
}

fn lcm(x: usize, y: usize) -> usize {
    x * y / gcf(x, y)
}

fn gcf(x: usize, y: usize) -> usize {
    let mut (num, denom) = if (x > y) { (x, y) } else { (y, x) };

    let mut rem;

    while {rem = num % denom; rem != 0} {
        num = denom;
        denom = rem;
    }

    denom
}
