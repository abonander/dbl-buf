use alloc::heap;

use math;

use std::{mem, ptr, slice};

const CACHE_LINE: usize = 64;

pub struct AlignedBufs<T> {
    ptr: *mut T,
    alloc_size: usize,
    bufb_offset: usize,
    cap: usize,
    len: usize,
}

impl<T> AlignedBufs<T> {
    pub fn alloc(cap: usize) -> Self {
        let (bufb_offset, alloc_size) = alloc_info::<T>(cap);

        let ptr = unsafe { 
            let ptr = heap::allocate(alloc_size, CACHE_LINE);

            assert!(!ptr.is_null(), "Failure to allocate");

            ptr
        };
        
        AlignedBufs {
            ptr: ptr as *mut T,
            alloc_size: alloc_size,
            bufb_offset: bufb_offset,
            cap: cap,
            len: 0,
        }
    } 

    fn realloc(&mut self, new_cap: usize) {
        let old_size = self.alloc_size;

        let (new_offset, new_size) = alloc_info::<T>(new_cap);        

        unsafe {
            // Move before the resize if it's going to shrink
            if new_size < old_size { 
                self.move_bufb(new_offset); 
            }

            let ptr = heap::reallocate(
                self.ptr as *mut u8,
                old_size,
                new_size,
                CACHE_LINE
            );
            
            assert!(!ptr.is_null(), "Failure to reallocate");

            self.ptr = ptr as *mut T;
            self.alloc_size = new_size;
            self.cap = new_cap;

            // Move after the resize if it's going to grow
            if new_size > old_size {
                self.move_bufb(new_offset);
            } 
        }
        
    }

    pub fn bufs(&mut self) -> (&mut [T], &mut [T]) {
        unsafe {
            (self.bufa(), self.bufb())
        }
    }

    pub fn bufa_ptr(&self) -> *mut T {
        self.ptr
    }

    pub unsafe fn bufa(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.ptr, self.len)
    } 

    pub fn bufb_ptr(&self) -> *mut T {
        (self.ptr as usize)
            .checked_add(self.bufb_offset)
            .unwrap() as *mut T
    }

    pub unsafe fn bufb(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufb_ptr(), self.len)
    }

    pub unsafe fn set_len(&mut self, len: usize)  {
        self.len = len;
    } 

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.len {
            return;
        }
        
        let bufa_ptr = self.ptr;
        let bufb_ptr = self.bufb_ptr();
    
        for new_len in (new_len .. self.len).rev() {
            let offset = new_len as isize;

            unsafe {
                self.set_len(new_len);

                let bufa_ptr = bufa_ptr.offset(offset);
                let bufb_ptr = bufb_ptr.offset(offset);

                ptr::drop_in_place(bufa_ptr);
                ptr::drop_in_place(bufb_ptr);
            }
        }
    }

    pub fn resize_with_fn<F: FnMut(usize, bool) -> T>(&mut self, new_len: usize, mut gen_fn: F) {
        if new_len < self.len {
            self.truncate(new_len);
            return;
        }
        
        if new_len > self.cap {
            self.realloc(new_len);
        }      

        let bufa_ptr = self.ptr;
        let bufb_ptr = self.bufb_ptr();

        for idx in self.len .. new_len {
            let bufa_val = gen_fn(new_len, false);
            let bufb_val = gen_fn(new_len, true);

            let offset = new_len as isize;

            unsafe {
                ptr::write(bufa_ptr.offset(offset), bufa_val);
                ptr::write(bufb_ptr.offset(offset), bufb_val);

                self.set_len(new_len + 1);
            }
        }
    }

    unsafe fn free(&mut self) {
        heap::deallocate(self.ptr as *mut u8, self.alloc_size, CACHE_LINE);
    }

    unsafe fn move_bufb(&mut self, new_offset: usize) {
        assert!(new_offset < self.alloc_size, "New offset is past alloc size");

        let bufb_oldptr = self.bufb_ptr();
        self.bufb_offset = new_offset;
        ptr::copy(bufb_oldptr, self.bufb_ptr(), self.len);
    } 
}

unsafe impl<T: Send> Send for AlignedBufs<T> {}
unsafe impl<T: Sync> Sync for AlignedBufs<T> {}

impl<T: Default> AlignedBufs<T> {
    pub fn resize(&mut self, new_len: usize) {
        self.resize_with_fn(new_len, |_, _| T::default())
    }
}

impl<T> Drop for AlignedBufs<T> {
    fn drop(&mut self) {
        self.truncate(0);

        unsafe {
           self.free();
        }
    }
}

fn size_of_alloc<T>(len: usize) -> usize {
    mem::size_of::<T>().checked_mul(len)
        .expect("Overflow calculating alloc size")
}

// Get the next multiple of `CACHE_LINE` up from `from`
fn next_cache_line(from: usize) -> usize {
    math::next_multiple(from, CACHE_LINE) 
}

/// Return the number of elements of `T` which will fit in some multiple
/// of the cache line size.
pub fn cache_aligned_len<T>() -> usize {
    math::lcm(CACHE_LINE, mem::size_of::<T>()) / mem::size_of::<T>()
}

// Get the bufb_offset and alloc_size for len 
fn alloc_info<T>(len: usize) -> (usize, usize) {
    assert!(
        mem::align_of::<T>() < CACHE_LINE, 
        "Mimimum alignment of type is larger than a cache line.
        How is this even possible?"
    );

    let buf_size = size_of_alloc::<T>(len);
    let bufb_offset = next_cache_line(buf_size);
    let alloc_size = bufb_offset + buf_size;

    (bufb_offset, alloc_size)
}

#[test]
fn test_next_cache_line() {
    assert_eq!(next_cache_line(16), 64);
    assert_eq!(next_cache_line(33), 64);
    assert_eq!(next_cache_line(95), 128);
    assert_eq!(next_cache_line(128), 192);
}

