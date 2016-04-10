use alloc::heap;

use math;

use std::{mem, ptr, slice};

const CACHE_LINE: usize = 64;

pub struct AlignedBufs<T> {
    bufa: *mut T,
    bufb: *mut T,
    len: usize,
}

impl<T> AlignedBufs<T> {
    pub fn alloc(len: usize) -> Self {
        let (bufb_offset, alloc_size) = alloc_info::<T>(len);

        let (bufa_ptr, bufb_ptr) = unsafe { 
            let ptr = heap::allocate(alloc_size, CACHE_LINE);

            assert!(!ptr.is_null(), "Failure to allocate");

            (ptr, ptr.offset(bufb_offset as isize))
        };
        
        AlignedBufs {
            bufa: bufa_ptr as *mut T,
            bufb: bufb_ptr as *mut T,
            len: len,
        }
    } 

    pub fn realloc(&mut self, new_len: usize) {
        let old_offset = self.bufb_offset();
        let old_size = self.alloc_size();

        let (new_offset, new_size) = alloc_info::<T>(new_len);        

        unsafe {
            // Move before the resize if it's going to shrink
            if new_len < self.len { 
                self.move_bufb(new_offset); 
            }

            let ptr = heap::reallocate(
                self.bufa as *mut u8,
                old_size,
                new_size,
                CACHE_LINE
            );

            assert!(!ptr.is_null(), "Failure to reallocate");

            self.bufa = ptr as *mut T;

            // Move after the resize if it's going to grow
            if new_len > self.len {
                self.bufb = ptr.offset(old_offset as isize) as *mut T;
                self.move_bufb(new_offset);
            } else {
                // Fixup the bufb pointer
                self.bufb = ptr.offset(new_offset as isize) as *mut T;
            }
        }
        
        self.len = new_len;
    }

    pub unsafe fn bufa(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufa, self.len)
    }

    pub unsafe fn bufb(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufb, self.len)        
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub unsafe fn drop_range(&mut self, from: usize, to: usize) {
        for val in &mut self.bufa()[from .. to] {
            ptr::drop_in_place(val);
        } 
        
        for val in &mut self.bufb()[from .. to] {
            ptr::drop_in_place(val);
        }
    }

    pub unsafe fn drop_all(&mut self) {
        for val in self.bufa() {
            ptr::drop_in_place(val);
        }

        for val in self.bufb() {
            ptr::drop_in_place(val);
        }
    }

    pub unsafe fn free(&mut self) {
        let alloc_size = self.alloc_size();

        heap::deallocate(self.bufa as *mut u8, alloc_size, CACHE_LINE);
    }

    unsafe fn move_bufb(&mut self, new_offset: usize) {
        let curr_offset = self.bufb_offset();
        let rel_offset = new_offset as isize - (curr_offset as isize);

        // Offset by byte count
        let bufb_newptr = (self.bufb as *mut u8).offset(rel_offset) as *mut T;

        ptr::copy(self.bufb, bufb_newptr, self.len);

        self.bufb = bufb_newptr;
    }

    fn bufb_offset(&self) -> usize {
        self.bufb as usize - (self.bufa as usize)
    }

    fn alloc_size(&self) -> usize {
        self.bufb as usize + self.bufb_offset()
    }
}

unsafe impl<T: Send> Send for AlignedBufs<T> {}
unsafe impl<T: Sync> Sync for AlignedBufs<T> {}

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

